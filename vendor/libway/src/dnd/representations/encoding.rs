//! Exact eager encodings with preallocation admission and shared equivalent byte owners.
use super::{Budget, Bytes, DragData};
use crate::{Error, Result};
use std::{os::unix::ffi::OsStrExt, path::PathBuf};

fn sum(mut capacities: impl Iterator<Item = usize>) -> Result<usize> {
    capacities
        .try_fold(0usize, |sum, capacity| sum.checked_add(capacity))
        .ok_or(Error::LimitExceeded)
}
fn paths_capacity(paths: &[PathBuf]) -> Result<usize> {
    sum(paths.iter().map(PathBuf::capacity))
}
pub(super) fn capacity(data: &DragData) -> Result<usize> {
    match data {
        DragData::Text(text) => Ok(text.capacity()),
        DragData::Files(paths) => paths_capacity(paths),
        DragData::FilesAndText { paths, text } => paths_capacity(paths)?
            .checked_add(text.capacity())
            .ok_or(Error::LimitExceeded),
        DragData::Static(entries) => sum(entries.iter().map(|(_, bytes)| bytes.capacity())),
        DragData::Lazy { .. } => Ok(0),
    }
}
pub(super) fn prepare(data: DragData, budget: &mut Budget) -> Result<Vec<(String, Bytes)>> {
    match data {
        DragData::Text(text) => text_entries(text, budget),
        DragData::Files(paths) => {
            let bytes = files(paths, budget)?;
            Ok(super::interop::FILE_SOURCE_MIMES
                .iter()
                .map(|mime| ((*mime).into(), bytes.clone()))
                .collect())
        }
        DragData::FilesAndText { paths, text } => {
            let bytes = files(paths, budget)?;
            let mut entries = vec![("text/uri-list".into(), bytes)];
            entries.extend(text_entries(text, budget)?);
            Ok(entries)
        }
        DragData::Static(entries) => static_entries(entries, budget),
        DragData::Lazy { .. } => Err(Error::InvalidInput(
            "eager encoding cannot prepare a Lazy provider",
        )),
    }
}
fn text_entries(text: String, budget: &mut Budget) -> Result<Vec<(String, Bytes)>> {
    budget.payload(text.len())?;
    let utf8 = Bytes::from(text.into_bytes());
    let latin1 = if utf8.is_ascii() {
        utf8.clone()
    } else {
        let text = std::str::from_utf8(&utf8)
            .expect("invariant: the text representation came from a UTF-8 String");
        let length = text.chars().count();
        budget.payload(length)?;
        budget.reserve(length)?;
        let mut bytes = Vec::with_capacity(length);
        bytes.extend(text.chars().map(|ch| u8::try_from(ch).unwrap_or(b'?')));
        Bytes::from(bytes)
    };
    Ok(super::interop::TEXT_SOURCE_MIMES
        .iter()
        .map(|mime| {
            let bytes = if *mime == "STRING" { &latin1 } else { &utf8 };
            ((*mime).into(), bytes.clone())
        })
        .collect())
}
fn files(paths: Vec<PathBuf>, budget: &mut Budget) -> Result<Bytes> {
    if paths.iter().any(|path| !path.is_absolute()) {
        return Err(Error::InvalidInput("dragged files need absolute paths"));
    }
    let length = paths
        .iter()
        .try_fold(0usize, |sum, path| {
            path.as_os_str()
                .as_bytes()
                .iter()
                .try_fold(sum.checked_add(9)?, |sum, &byte| {
                    sum.checked_add(if unescaped(byte) { 1 } else { 3 })
                })
        })
        .ok_or(Error::LimitExceeded)?;
    budget.payload(length)?;
    budget.reserve(length)?;
    let mut bytes = Vec::with_capacity(length);
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for path in &paths {
        bytes.extend_from_slice(b"file://");
        for &byte in path.as_os_str().as_bytes() {
            if unescaped(byte) {
                bytes.push(byte);
            } else {
                bytes.extend_from_slice(&[
                    b'%',
                    HEX[usize::from(byte >> 4)],
                    HEX[usize::from(byte & 15)],
                ]);
            }
        }
        bytes.extend_from_slice(b"\r\n");
    }
    let released = paths_capacity(&paths)?;
    drop(paths);
    budget.release(released);
    Ok(Bytes::from(bytes))
}
fn unescaped(byte: u8) -> bool {
    matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' |
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b',' | b';' | b'=' | b':' | b'@')
}
fn static_entries(
    entries: Vec<(String, Vec<u8>)>,
    budget: &mut Budget,
) -> Result<Vec<(String, Bytes)>> {
    let mut prepared: Vec<(String, Bytes)> = Vec::new();
    for (mime, bytes) in entries {
        budget.payload(bytes.len())?;
        if prepared.iter().any(|(name, _)| *name == mime) {
            budget.release(bytes.capacity());
            continue;
        }
        let shared = prepared.iter().find(|(_, old)| **old == bytes[..]);
        let bytes = if let Some((_, old)) = shared {
            budget.release(bytes.capacity());
            old.clone()
        } else {
            Bytes::from(bytes)
        };
        prepared.push((mime, bytes));
    }
    Ok(prepared)
}
