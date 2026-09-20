use std::io::{self, Read, Write};

use serde_json::Value;

const MAX_MESSAGE: usize = 32 * 1024;
pub const MAX_PREVIEW: usize = 32 * 1024 * 1024;

pub fn read_preview(reader: &mut impl Read, message: &Value) -> io::Result<Vec<u8>> {
    let length = message
        .get("preview_bytes")
        .map(|v| {
            v.as_u64()
                .ok_or_else(|| io::Error::other("invalid preview length"))
        })
        .transpose()?
        .unwrap_or(0);
    if length > MAX_PREVIEW as u64 {
        return Err(io::Error::other("replay preview is too large"));
    }
    let mut bytes = vec![0; length as usize];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn read(reader: &mut impl Read) -> io::Result<Value> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid replay message size",
        ));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub fn write(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(message)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(io::Error::other("replay message too large"));
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn command(message: &Value) -> Result<&str, String> {
    if message["version"].as_u64() != Some(1) {
        return Err("unsupported replay protocol version".into());
    }
    message["command"]
        .as_str()
        .ok_or_else(|| "missing replay command".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_reject_oversized_or_truncated_bodies() {
        let mut empty = std::io::Cursor::new(Vec::<u8>::new());
        assert!(
            read_preview(
                &mut empty,
                &serde_json::json!({"preview_bytes":MAX_PREVIEW+1})
            )
            .is_err()
        );
        assert!(read_preview(&mut empty, &serde_json::json!({"preview_bytes":1})).is_err());
        assert!(
            read_preview(&mut empty, &serde_json::json!({}))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn oversized_frames_and_unknown_versions_are_rejected() {
        assert!(read(&mut std::io::Cursor::new(u32::MAX.to_be_bytes())).is_err());
        assert!(command(&serde_json::json!({"version": 2, "command": "save"})).is_err());
        let value = serde_json::json!({"version": 1, "command": "status"});
        let mut encoded = Vec::new();
        write(&mut encoded, &value).unwrap();
        assert_eq!(read(&mut std::io::Cursor::new(encoded)).unwrap(), value);
    }
}
