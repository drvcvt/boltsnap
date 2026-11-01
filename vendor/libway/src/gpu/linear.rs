//! Tier 1: a LINEAR dma-buf read by the CPU through `mmap`, bracketed by
//! `DMA_BUF_IOCTL_SYNC`. No Vulkan is involved.
use crate::capture::ReadbackFallback;
use memmap2::MmapOptions;
use std::{
    os::fd::{AsRawFd, BorrowedFd},
    time::Instant,
};

/// `_IOW('b', 0, struct dma_buf_sync)` from `linux/dma-buf.h`.
const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;
const SYNC_READ: u64 = 1;
const SYNC_START: u64 = 0;
const SYNC_END: u64 = 4;

/// Where a LINEAR image sits in its dma-buf.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LinearLayout {
    pub offset: u32,
    pub stride: u32,
    pub width: u32,
    pub height: u32,
}

/// Read a completed LINEAR dma-buf as tightly packed `width * height * 4`
/// bytes: wait for its write fences until `end`, then copy the rows inside a
/// CPU access window.
pub(crate) fn read_linear(
    fd: BorrowedFd<'_>,
    layout: LinearLayout,
    end: Instant,
) -> Result<Vec<u8>, ReadbackFallback> {
    let invalid = || ReadbackFallback::Copy("invalid LINEAR layout".into());
    let row = layout.width as usize * 4;
    let (offset, stride) = (layout.offset as usize, layout.stride as usize);
    if layout.width == 0 || layout.height == 0 || stride < row {
        return Err(invalid());
    }
    let len = stride
        .checked_mul(layout.height as usize - 1)
        .and_then(|rows| rows.checked_add(offset))
        .and_then(|start| start.checked_add(row))
        .ok_or_else(invalid)?;
    wait_readable(fd, end)?;
    sync(fd, SYNC_START | SYNC_READ)?;
    let bytes = copy_rows(fd, len, offset, stride, row, layout.height as usize);
    // Ends the window on every path after it started.
    let ended = sync(fd, SYNC_END | SYNC_READ);
    let bytes = bytes?;
    ended?;
    Ok(bytes)
}

fn copy_rows(
    fd: BorrowedFd<'_>,
    len: usize,
    offset: usize,
    stride: usize,
    row: usize,
    height: usize,
) -> Result<Vec<u8>, ReadbackFallback> {
    // SAFETY: a read-only shared mapping of a dma-buf borrowed for the call.
    // The compositor finished writing (`ready`, then the implicit fences
    // waited above) and the CPU access window is open, so nothing writes it
    // while the slice exists; it is unmapped before returning.
    let map = unsafe { MmapOptions::new().len(len).map(fd.as_raw_fd()) }
        .map_err(|error| ReadbackFallback::Copy(format!("mmap of the dma-buf: {error}")))?;
    let mut bytes = Vec::with_capacity(row * height);
    for y in 0..height {
        let at = offset + y * stride;
        bytes.extend_from_slice(&map[at..at + row]);
    }
    Ok(bytes)
}

/// One `DMA_BUF_IOCTL_SYNC`, retried on `EINTR` and `EAGAIN`.
fn sync(fd: BorrowedFd<'_>, flags: u64) -> Result<(), ReadbackFallback> {
    loop {
        // SAFETY: the ioctl reads one `struct dma_buf_sync { __u64 flags; }`
        // from a valid pointer for the duration of the call.
        let result = unsafe { libc::ioctl(fd.as_raw_fd(), DMA_BUF_IOCTL_SYNC, &flags) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINTR | libc::EAGAIN)) {
            return Err(ReadbackFallback::Copy(format!(
                "DMA_BUF_IOCTL_SYNC: {error}"
            )));
        }
    }
}

/// Wait until the implicit write fences of a dma-buf signaled, or `end`.
pub(crate) fn wait_readable(fd: BorrowedFd<'_>, end: Instant) -> Result<(), ReadbackFallback> {
    loop {
        let left = end.saturating_duration_since(Instant::now());
        let ms = i32::try_from(left.as_nanos().div_ceil(1_000_000)).unwrap_or(i32::MAX);
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid `pollfd` for an open descriptor borrowed for the call.
        let ready = unsafe { libc::poll(&mut poll, 1, ms) };
        if ready > 0 {
            if poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err(ReadbackFallback::Copy("poll on the dma-buf failed".into()));
            }
            return Ok(());
        }
        if ready == 0 {
            return Err(ReadbackFallback::Timeout);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(ReadbackFallback::Copy(format!("poll: {error}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::fake::udmabuf;
    use std::{fs::File, os::fd::AsFd, time::Duration};

    fn end() -> Instant {
        Instant::now() + Duration::from_secs(1)
    }

    #[test]
    fn rows_are_packed_from_a_padded_dma_buf() {
        let Some(fd) = udmabuf(4096) else {
            eprintln!("skipped: /dev/udmabuf is not usable here");
            return;
        };
        let file = File::from(fd.try_clone().unwrap());
        // SAFETY: a private test buffer, written before it is read.
        let mut map = unsafe { MmapOptions::new().len(4096).map_mut(&file) }.unwrap();
        for (i, byte) in map.iter_mut().enumerate() {
            *byte = i as u8;
        }
        drop(map);
        let layout = LinearLayout {
            offset: 32,
            stride: 20,
            width: 3,
            height: 4,
        };
        let bytes = read_linear(fd.as_fd(), layout, end()).unwrap();
        let expected: Vec<u8> = (0..4)
            .flat_map(|y| (32 + y * 20..32 + y * 20 + 12).map(|i| i as u8))
            .collect();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn a_plain_memfd_is_no_dma_buf() {
        // SAFETY: constant name and flags; the result is checked.
        let raw = unsafe { libc::memfd_create(c"libway-plain".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(raw >= 0);
        // SAFETY: `raw` is a fresh descriptor owned by nobody else.
        let file = unsafe { <File as std::os::fd::FromRawFd>::from_raw_fd(raw) };
        file.set_len(4096).unwrap();
        let layout = LinearLayout {
            offset: 0,
            stride: 16,
            width: 4,
            height: 4,
        };
        let error = read_linear(file.as_fd(), layout, end()).unwrap_err();
        assert!(
            matches!(&error, ReadbackFallback::Copy(m) if m.contains("SYNC")),
            "{error}"
        );
    }

    #[test]
    fn layouts_that_do_not_fit_are_refused() {
        let Some(fd) = udmabuf(4096) else {
            eprintln!("skipped: /dev/udmabuf is not usable here");
            return;
        };
        for (offset, stride, width, height) in [
            (0, 8, 4, 1),
            (0, 16, 0, 1),
            (0, 16, 4, 0),
            (u32::MAX, u32::MAX, 4, u32::MAX),
            (4000, 64, 16, 2),
        ] {
            let layout = LinearLayout {
                offset,
                stride,
                width,
                height,
            };
            assert!(
                read_linear(fd.as_fd(), layout, end()).is_err(),
                "{layout:?}"
            );
        }
    }
}
