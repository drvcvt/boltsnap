//! Linux-only RGB8 transport. The JSON offer is nonmutating; only a sealed FD
//! sent after READY can create a shelf item. Never retry after sending that FD.
use serde_json::{Value, json};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

pub const COMMAND: &str = "add_pixels_v1";
const SEALS: i32 = libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Debug)]
pub struct Metadata {
    pub width: u32,
    pub height: u32,
    pub len: usize,
    pub source: String,
    pub output: Option<String>,
    pub copy: bool,
    pub trace: Option<u64>,
}
impl Metadata {
    pub fn new(
        width: u32,
        height: u32,
        source: String,
        output: Option<String>,
        copy: bool,
    ) -> io::Result<Self> {
        let pixels = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || pixels > (crate::protocol::MAX_PAYLOAD_BYTES / 4) as u64 {
            return Err(invalid(
                "RGB image exceeds the 64 Mi-pixel limit or is empty",
            ));
        }
        if source.len() > 128 || output.as_ref().is_some_and(|s| s.len() > 256) {
            return Err(invalid("image metadata is too long"));
        }
        Ok(Self {
            width,
            height,
            len: (pixels * 3) as usize,
            source,
            output,
            copy,
            trace: super::timing::request_id(),
        })
    }
    pub fn parse(value: &Value) -> io::Result<Self> {
        let number = |key| {
            value
                .get(key)
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid(format!("invalid {key}")))
        };
        if value["cmd"] != COMMAND || value["format"] != "RGB8" {
            return Err(invalid("unsupported pixel format"));
        }
        let width = u32::try_from(number("width")?).map_err(|_| invalid("width overflow"))?;
        let height = u32::try_from(number("height")?).map_err(|_| invalid("height overflow"))?;
        let source = value["source"]
            .as_str()
            .ok_or_else(|| invalid("invalid source"))?
            .to_owned();
        let output = match &value["output"] {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            _ => return Err(invalid("invalid output")),
        };
        let copy = value["copy"]
            .as_bool()
            .ok_or_else(|| invalid("invalid copy"))?;
        let mut result = Self::new(width, height, source, output, copy)?;
        if number("stride")? != u64::from(width) * 3 || number("length")? != result.len as u64 {
            return Err(invalid("RGB stride or length mismatch"));
        }
        result.trace = match &value["trace"] {
            Value::Null => None,
            v => Some(v.as_u64().ok_or_else(|| invalid("invalid trace ID"))?),
        };
        Ok(result)
    }
    pub fn header(&self) -> Vec<u8> {
        json!({"cmd":COMMAND,"format":"RGB8","width":self.width,"height":self.height,
            "stride":u64::from(self.width)*3,"length":self.len,"source":self.source,
            "output":self.output,"copy":self.copy,"trace":self.trace})
        .to_string()
        .into_bytes()
    }
}

pub fn same_user(stream: &UnixStream) -> io::Result<()> {
    // SAFETY: getsockopt writes a ucred into the correctly sized local object.
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
        if libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        ) != 0
        {
            return Err(io::Error::last_os_error());
        }
        if len as usize != std::mem::size_of_val(&cred) || cred.uid != libc::geteuid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "pixel peer must have the same UID",
            ));
        }
    }
    Ok(())
}

pub fn sealed_pixels(pixels: &[u8]) -> io::Result<OwnedFd> {
    // SAFETY: constant NUL-terminated name, returned fd has one owner.
    let fd = unsafe {
        libc::memfd_create(
            c"boltsnap-rgb".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(pixels)?;
    // No writable mapping exists. All future writes and size changes are denied.
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, SEALS) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file.into())
}

pub fn send_fd(stream: &UnixStream, fd: &OwnedFd) -> io::Result<()> {
    send_fds(stream, &[fd.as_raw_fd()])
}
fn send_fds(stream: &UnixStream, fds: &[RawFd]) -> io::Result<()> {
    assert!(fds.len() <= 16);
    // usize backing guarantees cmsghdr alignment. One payload byte makes a
    // successful send atomic for this protocol, so no FD is sent twice.
    let mut control = [0usize; 32];
    let mut marker = b'P';
    unsafe {
        let mut iov = libc::iovec {
            iov_base: (&mut marker as *mut u8).cast(),
            iov_len: 1,
        };
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(std::mem::size_of_val(fds) as u32) as usize;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(fds) as u32) as usize;
        std::ptr::copy_nonoverlapping(
            fds.as_ptr().cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            std::mem::size_of_val(fds),
        );
        loop {
            match libc::sendmsg(stream.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) {
                1 => return Ok(()),
                -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
                -1 => return Err(io::Error::last_os_error()),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "pixel FD not sent",
                    ));
                }
            }
        }
    }
}

pub fn receive_fd(stream: &UnixStream) -> io::Result<OwnedFd> {
    let mut control = [0usize; 8];
    let mut marker = 0u8;
    // SAFETY: aligned ancillary storage, bounded one-byte iovec. Kernel-created
    // descriptors are immediately owned, including extras on rejected messages.
    unsafe {
        let mut iov = libc::iovec {
            iov_base: (&mut marker as *mut u8).cast(),
            iov_len: 1,
        };
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        let received = loop {
            msg.msg_controllen = std::mem::size_of_val(&control);
            let n = libc::recvmsg(stream.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
            if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            break n;
        };
        let mut fds = Vec::new();
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*cmsg).cmsg_len.saturating_sub(libc::CMSG_LEN(0) as usize);
                for i in 0..bytes / std::mem::size_of::<RawFd>() {
                    let raw =
                        std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>().add(i));
                    fds.push(OwnedFd::from_raw_fd(raw));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
        if received != 1
            || marker != b'P'
            || msg.msg_flags & libc::MSG_CTRUNC != 0
            || fds.len() != 1
        {
            return Err(invalid("expected one complete pixel FD"));
        }
        Ok(fds.pop().unwrap())
    }
}

pub struct Pixels {
    address: std::ptr::NonNull<libc::c_void>,
    len: usize,
}
impl Pixels {
    pub fn map(fd: OwnedFd, len: usize) -> io::Result<Self> {
        if len == 0 || len > crate::protocol::MAX_PAYLOAD_BYTES {
            return Err(invalid("invalid mapping length"));
        }
        // SAFETY: fd is live. Requiring immutable size and contents before fstat
        // prevents truncation/SIGBUS and pixel changes after validation.
        unsafe {
            let seals = libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS);
            if seals < 0 || seals & SEALS != SEALS {
                return Err(invalid("pixel FD must be fully sealed"));
            }
            let mut stat: libc::stat = std::mem::zeroed();
            if libc::fstat(fd.as_raw_fd(), &mut stat) != 0 {
                return Err(io::Error::last_os_error());
            }
            if stat.st_mode & libc::S_IFMT != libc::S_IFREG || stat.st_size != len as i64 {
                return Err(invalid("pixel FD size or type mismatch"));
            }
            let address = libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            );
            if address == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            let Some(address) = std::ptr::NonNull::new(address) else {
                libc::munmap(address, len);
                return Err(invalid("null pixel mapping"));
            };
            Ok(Self { address, len })
        }
    }
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: immutable sealed mapping lives for self, bounded by fstat size.
        unsafe { std::slice::from_raw_parts(self.address.as_ptr().cast(), self.len) }
    }
}
impl Drop for Pixels {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.address.as_ptr(), self.len);
        }
    }
}

/// False means an older daemon closed the nonmutating offer. Any error after
/// READY is terminal: the caller must not add the image again via legacy IPC.
pub fn try_add(
    image: &image::RgbImage,
    source: &str,
    output: Option<String>,
    copy: bool,
) -> io::Result<bool> {
    let metadata = Metadata::new(image.width(), image.height(), source.into(), output, copy)?;
    let stream = super::ipc::ensure_daemon()?;
    transfer(stream, &metadata, image.as_raw())
}
fn transfer(mut stream: UnixStream, metadata: &Metadata, bytes: &[u8]) -> io::Result<bool> {
    same_user(&stream)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    crate::protocol::write_frame(&mut stream, &metadata.header(), &[])?;
    let (header, payload) = match crate::protocol::read_frame(&mut stream) {
        Ok(frame) => frame,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
            ) =>
        {
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    let response: Value = serde_json::from_slice(&header).map_err(|e| invalid(e.to_string()))?;
    if !payload.is_empty() || response["ready"] != COMMAND {
        return Err(invalid(
            response["error"]
                .as_str()
                .unwrap_or("invalid pixel READY response"),
        ));
    }
    let _timing = super::timing::Span::new("pixel_transfer_and_shelf_ack");
    let fd = sealed_pixels(bytes)?;
    send_fd(&stream, &fd).map_err(|e| {
        io::Error::other(format!(
            "pixel transfer failed; acceptance uncertain, do not retry automatically: {e}"
        ))
    })?;
    // PNG encoding/file preparation happens before ACK and can take longer on
    // a large image. Waiting does not create another worker or image allocation.
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    let response = crate::protocol::Response::read(&mut stream).map_err(|e| {
        io::Error::other(format!(
            "shelf ACK lost; image may have been accepted, do not retry automatically: {e}"
        ))
    })?;
    if !response.ok {
        return Err(io::Error::other(
            response
                .error
                .unwrap_or_else(|| "shelf rejected image".into()),
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_rejects_overflow_stride_and_oversize() {
        let m = Metadata::new(8, 7, "area".into(), None, false).unwrap();
        let value: Value = serde_json::from_slice(&m.header()).unwrap();
        assert_eq!(Metadata::parse(&value).unwrap().len, 168);
        for (key, v) in [
            ("width", json!(0)),
            ("width", json!(u64::MAX)),
            ("height", json!(u32::MAX)),
            ("stride", json!(25)),
            ("length", json!(169)),
            ("copy", json!(0)),
            ("format", json!("RGBA8")),
        ] {
            let mut bad = value.clone();
            bad[key] = v;
            assert!(Metadata::parse(&bad).is_err(), "{key}");
        }
    }
    #[test]
    fn fd_roundtrip_is_sealed_cloexec_and_exact() {
        let (a, b) = UnixStream::pair().unwrap();
        same_user(&a).unwrap();
        let fd = sealed_pixels(&[1, 2, 3, 4, 5, 6]).unwrap();
        send_fd(&a, &fd).unwrap();
        let received = receive_fd(&b).unwrap();
        assert_ne!(
            unsafe { libc::fcntl(received.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        let pixels = Pixels::map(received, 6).unwrap();
        assert_eq!(pixels.bytes(), [1, 2, 3, 4, 5, 6]);
        let mut file = File::from(fd);
        assert!(file.write_all(&[0]).is_err());
        assert!(file.set_len(0).is_err());
    }
    #[test]
    fn rejects_missing_extra_truncated_and_unsealed_fds() {
        for count in [0, 2, 16] {
            let (mut a, b) = UnixStream::pair().unwrap();
            let fd = sealed_pixels(&[1, 2, 3]).unwrap();
            if count == 0 {
                a.write_all(b"P").unwrap();
            } else {
                send_fds(&a, &vec![fd.as_raw_fd(); count]).unwrap();
            }
            assert!(receive_fd(&b).is_err());
            // Count references to this particular memfd inode, so parallel
            // tests opening unrelated descriptors cannot perturb the assertion.
            let inode = std::fs::metadata(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
            use std::os::unix::fs::MetadataExt;
            let references = std::fs::read_dir("/proc/self/fd")
                .unwrap()
                .filter_map(Result::ok)
                .filter_map(|entry| std::fs::metadata(entry.path()).ok())
                .filter(|m| m.ino() == inode.ino() && m.dev() == inode.dev())
                .count();
            assert_eq!(references, 1, "rejected ancillary FDs leaked");
        }
        let unsealed = unsafe {
            OwnedFd::from_raw_fd(libc::memfd_create(
                c"test".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            ))
        };
        assert!(Pixels::map(unsealed, 3).is_err());
        assert!(Pixels::map(sealed_pixels(&[1, 2, 3]).unwrap(), 4).is_err());
        let (a, _b) = UnixStream::pair().unwrap();
        assert!(Pixels::map(a.into(), 3).is_err());
    }
    #[test]
    fn future_write_seal_is_not_an_immutable_image() {
        let raw = unsafe {
            libc::memfd_create(
                c"test-future-write".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        assert!(raw >= 0);
        let mut file = unsafe { File::from_raw_fd(raw) };
        file.write_all(&[1, 2, 3]).unwrap();
        assert_eq!(
            unsafe {
                libc::fcntl(
                    raw,
                    libc::F_ADD_SEALS,
                    libc::F_SEAL_FUTURE_WRITE
                        | libc::F_SEAL_SHRINK
                        | libc::F_SEAL_GROW
                        | libc::F_SEAL_SEAL,
                )
            },
            0
        );
        assert!(Pixels::map(file.into(), 3).is_err());
    }

    #[test]
    fn legacy_eof_falls_back_but_lost_ack_does_not() {
        for ready in [false, true] {
            let (client, mut server) = UnixStream::pair().unwrap();
            let task = std::thread::spawn(move || {
                let _ = crate::protocol::read_frame(&mut server).unwrap();
                if ready {
                    crate::protocol::write_frame(
                        &mut server,
                        &json!({"ready":COMMAND}).to_string().into_bytes(),
                        &[],
                    )
                    .unwrap();
                    let _fd = receive_fd(&server).unwrap();
                }
            });
            let meta = Metadata::new(1, 1, "area".into(), None, false).unwrap();
            let result = transfer(client, &meta, &[8, 9, 10]);
            if ready {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("may have been accepted")
                );
            } else {
                assert!(!result.unwrap());
            }
            task.join().unwrap();
        }
    }
}
