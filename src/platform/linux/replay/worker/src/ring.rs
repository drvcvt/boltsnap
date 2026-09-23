use crate::replay::{ring::Entry, time::micros};
use ffmpeg_next::{Packet, Rational, packet::Ref};
use std::mem::size_of;
use std::sync::Arc;

pub fn entry(packet: Packet, time_base: Rational) -> Result<Entry<Arc<Packet>>, String> {
    if packet.is_corrupt() {
        return Err("corrupt input packet".into());
    }
    let pts = packet.pts().ok_or("packet has no PTS")?;
    let dts = packet.dts().ok_or("packet has no DTS")?;
    if pts != dts {
        return Err("probe requires a stream without frame reordering".into());
    }
    if packet.duration() <= 0 {
        return Err("probe requires positive packet durations".into());
    }
    let end = pts
        .checked_add(packet.duration())
        .ok_or("timestamp overflow")?;
    let start_us = micros(pts, time_base.0, time_base.1)?;
    let end_us = micros(end, time_base.0, time_base.1)?;
    if end_us <= start_us {
        return Err("packet duration is below clock resolution".into());
    }
    // The packet owns its buffer reference and side-data descriptors.
    let buffer_bytes = unsafe {
        let buffer = (*packet.as_ptr()).buf;
        if buffer.is_null() {
            packet.size()
        } else {
            // Older FFmpeg versions use a signed buffer size.
            #[allow(clippy::useless_conversion)]
            let size = usize::try_from((*buffer).size).map_err(|_| "invalid packet buffer size")?;
            size
        }
    };
    let side_bytes = packet.side_data().try_fold(0_usize, |sum, data| {
        sum.checked_add(data.data().len())
            .and_then(|n| n.checked_add(size_of::<ffmpeg_next::ffi::AVPacketSideData>() + 64))
            .ok_or("packet accounting overflow")
    })?;
    // Keep the existing conservative allowance: it also covers the Arc
    // allocation and both live/frozen Entry descriptors.
    let bytes = buffer_bytes
        .checked_add(side_bytes)
        .and_then(|n| n.checked_add(2 * size_of::<Entry<Packet>>() + 128))
        .ok_or("packet accounting overflow")?;
    Ok(Entry {
        keyframe: packet.is_key(),
        // Snapshots share immutable headers as well as encoded data. Export
        // takes its own AVPacket reference before modifying timestamps.
        payload: Arc::new(packet),
        start_us,
        end_us,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet() -> Packet {
        let mut packet = Packet::copy(&[1, 2, 3]);
        packet.set_pts(Some(0));
        packet.set_dts(Some(0));
        packet.set_duration(1);
        packet
    }

    #[test]
    fn snapshot_keeps_immutable_packet_after_live_eviction() {
        let mut input = packet();
        input.set_flags(ffmpeg_next::packet::Flags::KEY);
        let original = entry(input, Rational(1, 1)).unwrap();
        let mut live = crate::replay::ring::Ring::new(1_000_000, 1024 * 1024).unwrap();
        live.push(original, true).unwrap();
        let frozen = live.snapshot();
        assert!(Arc::ptr_eq(
            &live.video().next().unwrap().payload,
            &frozen.video().next().unwrap().payload
        ));
        let mut next = packet();
        next.set_pts(Some(1));
        next.set_dts(Some(1));
        next.set_flags(ffmpeg_next::packet::Flags::KEY);
        live.push(entry(next, Rational(1, 1)).unwrap(), true)
            .unwrap();
        assert_eq!(live.bounds().unwrap(), (1_000_000, 2_000_000));
        assert_eq!(frozen.bounds().unwrap(), (0, 1_000_000));
        let retained = &frozen.video().next().unwrap().payload;
        assert_eq!(retained.data(), Some(&[1, 2, 3][..]));
        assert_eq!(retained.pts(), Some(0));
        assert_eq!(Arc::strong_count(retained), 1);
    }

    #[test]
    fn invalid_timing_is_rejected_before_buffering() {
        let mut input = packet();
        input.set_pts(Some(1));
        assert!(entry(input, Rational(1, 60)).is_err());
        let mut input = packet();
        input.set_dts(None);
        assert!(entry(input, Rational(1, 60)).is_err());
        let mut input = packet();
        input.set_duration(0);
        assert!(entry(input, Rational(1, 60)).is_err());
    }
}
