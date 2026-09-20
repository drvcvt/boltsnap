use std::ffi::CStr;
use std::fs::File;
use std::os::fd::FromRawFd;
use std::path::Path;
use std::sync::Arc;

use ffmpeg_next::{
    self as ffmpeg, Packet, Rational, codec, format, media,
    packet::{Mut, Ref},
};

use crate::replay::ring::Ring;

pub struct Stream {
    pub parameters: codec::Parameters,
    pub time_base: Rational,
    pub frame_rate: Rational,
}

pub struct Recording {
    pub streams: Vec<Stream>,
    pub ring: Ring<Arc<Packet>>,
    pub packets_read: u64,
}

pub fn open(path: &Path) -> Result<(format::context::Input, Vec<Stream>, usize), String> {
    let file = if path == Path::new("-") {
        // The worker consumes stdin once; closing the demuxer closes its pipe.
        unsafe { File::from_raw_fd(libc::STDIN_FILENO) }
    } else {
        File::open(path).map_err(|e| format!("open input: {e}"))?
    };
    let io =
        format::context::StreamIo::from_read(file).map_err(|e| format!("input stream: {e}"))?;
    let mut options = ffmpeg::Dictionary::new();
    options.set("probesize", "1048576");
    // Long enough for libavformat to settle the video stream's decode delay.
    // Below roughly 200 ms it gives up, and then it cannot compute a DTS for the
    // first packets and emits AV_NOPTS_VALUE, which the ring rejects outright.
    options.set("analyzeduration", "1000000");
    options.set("max_probe_packets", "128");
    options.set("indexmem", "65536");
    options.set("format_whitelist", "nut,matroska,webm");
    let input = format::input_from_stream(io, None, Some(options))
        .map_err(|e| format!("demux input: {e}"))?;
    if input.nb_streams() == 0 || input.nb_streams() > 2 {
        return Err("probe requires one video and at most one audio stream".into());
    }
    let mut streams = Vec::new();
    let mut video_index = None;
    for stream in input.streams() {
        match stream.parameters().medium() {
            media::Type::Video if video_index.is_none() => video_index = Some(stream.index()),
            media::Type::Audio => {}
            _ => return Err("probe requires one video and at most one audio stream".into()),
        }
        let mut parameters = codec::Parameters::new();
        // Allocation failure must be checked before passing the destination to FFmpeg.
        if unsafe { parameters.as_ptr().is_null() } {
            return Err("allocate stream parameters".into());
        }
        // Both parameter blocks are live; the destination is independently owned.
        let result = unsafe {
            ffmpeg::ffi::avcodec_parameters_copy(
                parameters.as_mut_ptr(),
                stream.parameters().as_ptr(),
            )
        };
        if result < 0 {
            return Err(format!(
                "copy stream parameters: {}",
                ffmpeg::Error::from(result)
            ));
        }
        streams.push(Stream {
            parameters,
            time_base: stream.time_base(),
            frame_rate: if stream.avg_frame_rate().numerator() > 0 {
                stream.avg_frame_rate()
            } else {
                stream.rate()
            },
        });
    }
    let video_index = video_index.ok_or("input has no video stream")?;
    Ok((input, streams, video_index))
}

pub fn ingest(path: &Path, duration_us: i64, budget: usize) -> Result<Recording, String> {
    let (mut input, streams, video_index) = open(path)?;
    let mut ring = Ring::new(duration_us, budget)?;
    let mut packets_read = 0_u64;
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => return Err(format!("read packet: {error}")),
        }
        let index = packet.stream();
        let stream = streams
            .get(index)
            .ok_or("packet references an unknown stream")?;
        let entry = crate::ring::entry(packet, stream.time_base)?;
        ring.push(entry, index == video_index)?;
        packets_read = packets_read.checked_add(1).ok_or("packet count overflow")?;
    }
    ring.bounds()?;
    Ok(Recording {
        streams,
        ring,
        packets_read,
    })
}

pub fn reference(packet: &Packet) -> Result<Packet, String> {
    let mut referenced = Packet::empty();
    // Only the packet header is copied; payload buffers remain refcounted.
    let result = unsafe { ffmpeg::ffi::av_packet_ref(referenced.as_mut_ptr(), packet.as_ptr()) };
    if result < 0 {
        return Err(format!("reference packet: {}", ffmpeg::Error::from(result)));
    }
    Ok(referenced)
}

pub fn capabilities() -> serde_json::Value {
    let mut encoders = Vec::new();
    let mut cursor = std::ptr::null_mut();
    loop {
        // FFmpeg returns immutable codec descriptors with process lifetime.
        unsafe {
            let codec = ffmpeg::ffi::av_codec_iterate(&mut cursor);
            if codec.is_null() {
                break;
            }
            if ffmpeg::ffi::av_codec_is_encoder(codec) == 0
                || (*codec).type_ != ffmpeg::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO
            {
                continue;
            }
            encoders.push(CStr::from_ptr((*codec).name).to_string_lossy().into_owned());
        }
    }
    encoders.sort();
    serde_json::json!({"protocol_version": 1, "mode": "probe",
        "libavformat_version": format::version(), "video_encoders": encoders,
        "hardware_probed": false})
}

impl Recording {
    pub fn snapshot(&self) -> Result<Self, String> {
        self.snapshot_with_ring(
            self.ring
                .snapshot(|packet| Ok::<_, String>(Arc::clone(packet)))?,
        )
    }

    pub fn tail_snapshot(&self) -> Result<Self, String> {
        self.snapshot_with_ring(
            self.ring
                .tail_snapshot(|packet| Ok::<_, String>(Arc::clone(packet)))?,
        )
    }

    fn snapshot_with_ring(&self, ring: Ring<Arc<Packet>>) -> Result<Self, String> {
        self.ring.bounds()?;
        let streams = self
            .streams
            .iter()
            .map(|stream| {
                let mut parameters = codec::Parameters::new();
                if unsafe { parameters.as_ptr().is_null() } {
                    return Err("allocate snapshot parameters".into());
                }
                let result = unsafe {
                    ffmpeg::ffi::avcodec_parameters_copy(
                        parameters.as_mut_ptr(),
                        stream.parameters.as_ptr(),
                    )
                };
                if result < 0 {
                    return Err(format!(
                        "copy snapshot parameters: {}",
                        ffmpeg::Error::from(result)
                    ));
                }
                Ok(Stream {
                    parameters,
                    time_base: stream.time_base,
                    frame_rate: stream.frame_rate,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            streams,
            ring,
            packets_read: self.packets_read,
        })
    }

    pub fn dimensions(&self) -> Result<(u32, u32), String> {
        let stream = self
            .streams
            .iter()
            .find(|s| s.parameters.medium() == media::Type::Video)
            .ok_or("no video stream")?;
        let parameters = unsafe { &*stream.parameters.as_ptr() };
        let width = u32::try_from(parameters.width).map_err(|_| "invalid video width")?;
        let height = u32::try_from(parameters.height).map_err(|_| "invalid video height")?;
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err("unsupported video dimensions".into());
        }
        Ok((width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_reference_shares_payload_and_keeps_independent_timestamps() {
        let mut original = Packet::copy(&[1, 2, 3]);
        original.set_pts(Some(42));
        let mut copy = reference(&original).unwrap();
        assert_eq!(
            original.data().unwrap().as_ptr(),
            copy.data().unwrap().as_ptr()
        );
        copy.set_pts(Some(0));
        assert_eq!(original.pts(), Some(42));
        drop(original);
        assert_eq!(copy.data().unwrap(), &[1, 2, 3]);
    }
}
