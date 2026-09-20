use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ffmpeg_next::{self as ffmpeg, Rational, Rescale, format};

use crate::media::{Recording, reference};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct LimitedFile {
    file: File,
    limit: u64,
}

impl Write for LimitedFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .file
            .stream_position()?
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("output size overflow"))?;
        if end > self.limit {
            return Err(io::Error::other("probe output quota exceeded"));
        }
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for LimitedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

fn create_partial(path: &Path, limit: u64) -> Result<(PathBuf, LimitedFile, File), String> {
    let name = path.file_name().ok_or("output has no filename")?;
    for _ in 0..32 {
        let mut temporary = name.to_os_string();
        temporary.push(format!(
            ".{}.{}.partial",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = path.with_file_name(temporary);
        match OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => {
                let handle = file
                    .try_clone()
                    .map_err(|e| format!("retain output handle: {e}"))?;
                return Ok((temporary, LimitedFile { file, limit }, handle));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create output: {error}")),
        }
    }
    Err("could not reserve a temporary output filename".into())
}

pub(crate) fn publish(source: &Path, destination: &Path) -> Result<(), String> {
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| "NUL in source path")?;
    let destination =
        CString::new(destination.as_os_str().as_bytes()).map_err(|_| "NUL in output path")?;
    // Both paths are NUL-terminated; RENAME_NOREPLACE preserves an existing destination.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(format!("publish output: {}", io::Error::last_os_error()));
    }
    Ok(())
}

pub fn remux(recording: &Recording, path: &Path, limit: u64) -> Result<u64, String> {
    if path.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err("probe output must use the .mkv extension".into());
    }
    if path
        .try_exists()
        .map_err(|e| format!("check output: {e}"))?
    {
        return Err("output already exists".into());
    }
    let (temporary, writer, handle) = create_partial(path, limit)?;
    let result: Result<u64, String> = (|| {
        mux(recording, writer)?;
        handle.sync_all().map_err(|e| format!("sync output: {e}"))?;
        let bytes = handle
            .metadata()
            .map_err(|e| format!("output metadata: {e}"))?
            .len();
        if bytes == 0 {
            return Err("empty output".into());
        }
        publish(&temporary, path)?;
        Ok(bytes)
    })();
    result.map_err(|e| format!("{e}; partial output: {}", temporary.display()))
}

pub fn to_file(recording: &Recording, file: File, limit: u64) -> Result<(), String> {
    mux(recording, LimitedFile { file, limit })
}

fn mux(recording: &Recording, writer: LimitedFile) -> Result<(), String> {
    let (start, _) = recording.ring.bounds()?;
    let io = format::context::StreamIo::from_write_seek(writer)
        .map_err(|e| format!("output stream: {e}"))?;
    let mut output = format::output_to_stream(io, None, Some("matroska"))
        .map_err(|e| format!("output container: {e}"))?;
    for stream in &recording.streams {
        let mut target = output
            .add_stream(ffmpeg::encoder::find(ffmpeg::codec::Id::None))
            .map_err(|e| format!("add output stream: {e}"))?;
        target.set_time_base(stream.time_base);
        target.set_rate(stream.frame_rate);
        target.set_avg_frame_rate(stream.frame_rate);
        // The output owns the destination; container tags are regenerated.
        let result = unsafe {
            let destination = target.parameters().as_mut_ptr();
            if destination.is_null() {
                return Err("allocate output parameters".into());
            }
            let result =
                ffmpeg::ffi::avcodec_parameters_copy(destination, stream.parameters.as_ptr());
            (*destination).codec_tag = 0;
            result
        };
        if result < 0 {
            return Err(format!(
                "copy output parameters: {}",
                ffmpeg::Error::from(result)
            ));
        }
    }
    output
        .write_header()
        .map_err(|e| format!("write header: {e}"))?;
    let mut video = recording.ring.video().peekable();
    let mut audio = recording.ring.audio().peekable();
    loop {
        let entry = match (video.peek(), audio.peek()) {
            (Some(v), Some(a)) if a.start_us < v.start_us => audio.next(),
            (Some(_), _) => video.next(),
            (_, Some(_)) => audio.next(),
            _ => break,
        }
        .expect("at least one stream has packets");
        let mut packet = reference(&entry.payload)?;
        let index = packet.stream();
        let input_base = recording.streams[index].time_base;
        let offset = start.rescale(Rational(1, 1_000_000), input_base);
        packet.set_pts(Some(
            packet
                .pts()
                .ok_or("missing PTS")?
                .checked_sub(offset)
                .ok_or("PTS overflow")?,
        ));
        packet.set_dts(Some(
            packet
                .dts()
                .ok_or("missing DTS")?
                .checked_sub(offset)
                .ok_or("DTS overflow")?,
        ));
        let time_base = output
            .stream(index)
            .ok_or("missing output stream")?
            .time_base();
        packet.rescale_ts(input_base, time_base);
        packet.set_position(-1);
        packet
            .write_interleaved(&mut output)
            .map_err(|e| format!("write packet: {e}"))?;
    }
    output
        .write_trailer()
        .map_err(|e| format!("write trailer: {e}"))?;
    // Custom AVIO belongs to output and remains live through the flush.
    let error = unsafe {
        let io = (*output.as_mut_ptr()).pb;
        ffmpeg::ffi::avio_flush(io);
        (*io).error
    };
    if error < 0 {
        return Err(format!("flush output: {}", ffmpeg::Error::from(error)));
    }
    drop(output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_and_late_destination_collision_preserve_files() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("boltsnap-export-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let destination = directory.join("clip.mkv");
        let (partial, mut writer, handle) = create_partial(&destination, 3).unwrap();
        writer.write_all(b"new").unwrap();
        assert!(writer.write_all(b"!").is_err());
        assert_eq!(handle.metadata().unwrap().len(), 3);
        std::fs::write(&destination, b"original").unwrap();
        assert!(publish(&partial, &destination).is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"original");
        assert_eq!(std::fs::read(partial).unwrap(), b"new");
    }
}
