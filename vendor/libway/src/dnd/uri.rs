//! `text/uri-list` (RFC 2483) with `file:` URIs (RFC 8089). Paths stay lossless byte strings;
//! consumer policy (UTF-8 only, no control characters) belongs to the consumer.
use super::interop::PayloadKind;
use crate::TransferError;
use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

fn hex(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).map(|d| d as u8)
}
/// `+` stays `+` (no form decoding); raw spaces, controls, `?` and `#` are rejected instead of
/// being cut off silently.
fn percent_decode(path: &[u8]) -> Result<Vec<u8>, TransferError> {
    let mut out = Vec::with_capacity(path.len());
    let mut i = 0;
    while i < path.len() {
        match path[i] {
            b'%' => {
                let digit = |at: usize| path.get(at).copied().and_then(hex);
                let (Some(h), Some(l)) = (digit(i + 1), digit(i + 2)) else {
                    return Err(TransferError::Malformed);
                };
                out.push((h << 4) | l);
                i += 3;
            }
            0x00..=0x20 | 0x7f | b'?' | b'#' => return Err(TransferError::Malformed),
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    if out.contains(&0) {
        return Err(TransferError::Malformed);
    }
    Ok(out)
}
/// No authority, `localhost`, or this machine's name: `ls --hyperlink` and many file
/// managers write `file://$HOSTNAME/path`. Any other host names a file elsewhere.
fn is_local_host(host: &[u8]) -> bool {
    if host.is_empty() || host.eq_ignore_ascii_case(b"localhost") {
        return true;
    }
    let mut name = [0u8; 256];
    // SAFETY: the buffer outlives the call and its length is passed.
    if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } != 0 {
        return false;
    }
    let len = name.iter().position(|b| *b == 0).unwrap_or(name.len());
    len > 0 && host.eq_ignore_ascii_case(&name[..len])
}
fn parse_file_uri(line: &[u8]) -> Result<PathBuf, TransferError> {
    let rest = line
        .strip_prefix(b"file:")
        .ok_or(TransferError::Malformed)?;
    let path = match rest.strip_prefix(b"//") {
        Some(after) => {
            let slash = after.iter().position(|b| *b == b'/');
            let slash = slash.ok_or(TransferError::Malformed)?;
            let host = &after[..slash];
            if !is_local_host(host) {
                return Err(TransferError::Malformed);
            }
            &after[slash..]
        }
        None if rest.first() == Some(&b'/') => rest,
        None => return Err(TransferError::Malformed),
    };
    Ok(PathBuf::from(OsString::from_vec(percent_decode(path)?)))
}
/// All entries or nothing: one foreign or malformed URI fails the whole list.
pub(crate) fn parse_uri_list(
    bytes: &[u8],
    max_entries: usize,
) -> Result<Vec<PathBuf>, TransferError> {
    let mut paths = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        if paths.len() >= max_entries {
            return Err(TransferError::TooLarge);
        }
        paths.push(parse_file_uri(line)?);
    }
    Ok(paths)
}
pub(crate) fn encode_uri_list(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for path in paths {
        out.extend_from_slice(b"file://");
        for &b in path.as_os_str().as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                    out.push(b)
                }
                b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b',' | b';' | b'=' | b':'
                | b'@' => out.push(b),
                _ => out.extend_from_slice(format!("%{b:02X}").as_bytes()),
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}
pub(crate) fn decode_text(bytes: &[u8], kind: PayloadKind) -> Result<String, TransferError> {
    match kind {
        PayloadKind::Latin1Text => Ok(bytes.iter().map(|b| char::from(*b)).collect()),
        _ => String::from_utf8(bytes.to_vec()).map_err(|_| TransferError::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }
    #[test]
    fn parses_crlf_lf_comments_and_percent_escapes_in_order() {
        let input = b"# comment\r\nfile:///tmp/a%20b.txt\r\n\r\nfile://localhost/x/%25/%2B\nfile:///e%C3%A9\nfile:/single\n";
        assert_eq!(
            parse_uri_list(input, 10).unwrap(),
            vec![p("/tmp/a b.txt"), p("/x/%/+"), p("/e\u{e9}"), p("/single")]
        );
    }
    #[test]
    fn rejects_foreign_hosts_schemes_relative_paths_bad_escapes_and_nul() {
        for bad in [
            &b"file://nas/share/x"[..],
            b"file://host",
            b"http://example.com/x",
            b"file:relative",
            b"file:///bad%zz",
            b"file:///bad%2",
            b"file:///nul%00byte",
            b"file:///a?b",
            b"file:///a#b",
            b"file:///with space",
            b"file:///tab\there",
            b"/plain/path",
        ] {
            assert_eq!(
                parse_uri_list(bad, 10),
                Err(TransferError::Malformed),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        let one_bad = b"file:///ok\nhttp://x/y\n";
        assert_eq!(parse_uri_list(one_bad, 10), Err(TransferError::Malformed));
    }
    #[test]
    fn this_machines_host_name_is_a_local_authority() {
        let mut name = [0u8; 256];
        unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) };
        let host = String::from_utf8_lossy(&name[..name.iter().position(|b| *b == 0).unwrap()])
            .into_owned();
        let uri = format!("file://{}/tmp/a%20b", host.to_ascii_uppercase());
        assert_eq!(
            parse_uri_list(uri.as_bytes(), 1).unwrap(),
            vec![p("/tmp/a b")]
        );
        let remote = b"file://not-this-host.invalid/tmp/x";
        assert_eq!(parse_uri_list(remote, 1), Err(TransferError::Malformed));
    }
    #[test]
    fn enforces_entry_limit_and_keeps_non_utf8_bytes_lossless() {
        let three = b"file:///a\nfile:///b\nfile:///c\n";
        assert_eq!(parse_uri_list(three, 2), Err(TransferError::TooLarge));
        let paths = parse_uri_list(b"file:///%FF", 1).unwrap();
        assert_eq!(paths[0].as_os_str().as_bytes(), b"/\xff");
        assert!(parse_uri_list(b"", 1).unwrap().is_empty());
        assert!(
            parse_uri_list(b"# only a comment\r\n", 0)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn encodes_reserved_bytes_and_round_trips() {
        let raw = PathBuf::from(OsString::from_vec(b"/raw\xff\n".to_vec()));
        let paths = vec![p("/tmp/a b"), p("/x/%/+#?"), p("/e\u{e9}"), raw];
        let bytes = encode_uri_list(&paths);
        assert_eq!(
            bytes,
            b"file:///tmp/a%20b\r\nfile:///x/%25/%2B%23%3F\r\nfile:///e%C3%A9\r\nfile:///raw%FF%0A\r\n"
        );
        assert_eq!(parse_uri_list(&bytes, 10).unwrap(), paths);
    }
    #[test]
    fn text_decoding_handles_utf8_and_latin1_and_rejects_invalid_utf8() {
        assert_eq!(
            decode_text(b"h\xc3\xa9", PayloadKind::Utf8Text).unwrap(),
            "h\u{e9}"
        );
        assert_eq!(
            decode_text(b"h\xe9", PayloadKind::Latin1Text).unwrap(),
            "h\u{e9}"
        );
        assert_eq!(
            decode_text(b"h\xe9", PayloadKind::Utf8Text),
            Err(TransferError::Malformed)
        );
        assert_eq!(
            decode_text(b"x\r\ny", PayloadKind::Utf8Text).unwrap(),
            "x\r\ny"
        );
    }
}
