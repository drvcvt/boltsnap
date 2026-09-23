//! MIME/action mapping shared with X11 clients bridged through XWayland. Compositors
//! translate X11 target atoms to their names, so `UTF8_STRING`/`TEXT`/`STRING` appear verbatim.
use super::{Accept, DragData};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PayloadKind {
    Files,
    Utf8Text,
    Latin1Text,
    Bytes,
}
pub(crate) const FILE_MIMES: &[&str] = &["text/uri-list"];
pub(crate) const TEXT_MIMES: &[(&str, PayloadKind)] = &[
    ("text/plain;charset=utf-8", PayloadKind::Utf8Text),
    ("UTF8_STRING", PayloadKind::Utf8Text),
    ("text/plain", PayloadKind::Utf8Text),
    ("TEXT", PayloadKind::Utf8Text),
    ("STRING", PayloadKind::Latin1Text),
];
/// First accept entry that the offer can satisfy, in the consumer's preference order.
pub(crate) fn choose_mime(offered: &[String], accepts: &[Accept]) -> Option<(String, PayloadKind)> {
    choices(offered, accepts).into_iter().next()
}
/// Every offered type the consumer accepts, best first: consumer order, then each accept's own
/// preference. A drop whose first choice fails to decode falls back along this list.
pub(crate) fn choices(offered: &[String], accepts: &[Accept]) -> Vec<(String, PayloadKind)> {
    let has = |m: &str| offered.iter().any(|o| o == m);
    let mut out: Vec<(String, PayloadKind)> = Vec::new();
    for accept in accepts {
        let found: Vec<(String, PayloadKind)> = match accept {
            Accept::Files => (FILE_MIMES.iter().filter(|m| has(m)))
                .map(|m| (m.to_string(), PayloadKind::Files))
                .collect(),
            Accept::Text => (TEXT_MIMES.iter().filter(|(m, _)| has(m)))
                .map(|(m, kind)| (m.to_string(), *kind))
                .collect(),
            Accept::Mime(list) => (list.iter().filter(|m| has(m)))
                .map(|m| (m.clone(), PayloadKind::Bytes))
                .collect(),
        };
        for choice in found {
            if !out.iter().any(|(m, _)| *m == choice.0) {
                out.push(choice);
            }
        }
    }
    out
}

pub(crate) const TEXT_SOURCE_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "TEXT",
    "STRING",
];
/// Files also go out as text so targets behind an XDND bridge that only take text get the list.
pub(crate) const FILE_SOURCE_MIMES: &[&str] =
    &["text/uri-list", "text/plain;charset=utf-8", "text/plain"];
pub(crate) fn source_mimes(data: &DragData) -> Vec<String> {
    let fixed = |list: &[&str]| list.iter().map(|m| m.to_string()).collect();
    match data {
        DragData::Files(_) => fixed(FILE_SOURCE_MIMES),
        DragData::Text(_) => fixed(TEXT_SOURCE_MIMES),
        DragData::FilesAndText { .. } => {
            let mut mimes = fixed(FILE_MIMES);
            mimes.extend(fixed(TEXT_SOURCE_MIMES));
            mimes
        }
        DragData::Static(v) => v.iter().map(|(m, _)| m.clone()).collect(),
        DragData::Lazy { mimes, .. } => mimes.clone(),
    }
}
/// Bytes for one requested type; `STRING` is Latin-1 for X11 targets, `?` for the rest.
/// Only announced types are answered; fixed type names compare without ASCII case, as some
/// targets request `text/plain;charset=UTF-8`.
pub(crate) fn materialize(data: &mut DragData, mime: &str) -> Option<Arc<[u8]>> {
    let one_of = |list: &[&str]| list.iter().any(|m| m.eq_ignore_ascii_case(mime));
    match data {
        DragData::Files(paths) if one_of(FILE_SOURCE_MIMES) => {
            Some(super::uri::encode_uri_list(paths).into())
        }
        DragData::Files(_) => None,
        DragData::FilesAndText { paths, .. } if one_of(FILE_MIMES) => {
            Some(super::uri::encode_uri_list(paths).into())
        }
        DragData::Text(t) | DragData::FilesAndText { text: t, .. } => {
            if !one_of(TEXT_SOURCE_MIMES) {
                return None;
            }
            if mime.eq_ignore_ascii_case("STRING") {
                let latin1 = t.chars().map(|c| u8::try_from(c).unwrap_or(b'?'));
                return Some(latin1.collect::<Vec<_>>().into());
            }
            Some(t.as_bytes().into())
        }
        DragData::Static(v) => v.iter().find(|(m, _)| m == mime).map(|(_, b)| b[..].into()),
        DragData::Lazy { mimes, provide } => mimes
            .iter()
            .any(|m| m == mime)
            .then(|| provide(mime))?
            .map(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    #[test]
    fn consumer_order_wins_over_offer_order() {
        let offered = s(&["text/plain", "text/uri-list"]);
        let text_first = choose_mime(&offered, &[Accept::Text, Accept::Files]).unwrap();
        assert_eq!(text_first.0, "text/plain");
        let files_first = choose_mime(&offered, &[Accept::Files, Accept::Text]).unwrap();
        assert_eq!(files_first.0, "text/uri-list");
        let raw = choose_mime(&s(&["image/png"]), &[Accept::Mime(s(&["image/png"]))]).unwrap();
        assert_eq!(raw, ("image/png".into(), PayloadKind::Bytes));
    }
    #[test]
    fn choices_list_every_accepted_type_best_first_without_duplicates() {
        let offered = s(&["text/plain", "text/uri-list", "UTF8_STRING", "image/png"]);
        let accepts = [
            Accept::Files,
            Accept::Text,
            Accept::Mime(s(&["image/png", "text/plain"])),
        ];
        let mimes: Vec<String> = choices(&offered, &accepts)
            .into_iter()
            .map(|c| c.0)
            .collect();
        assert_eq!(
            mimes,
            ["text/uri-list", "UTF8_STRING", "text/plain", "image/png"]
        );
        assert!(choices(&offered, &[]).is_empty());
    }
    #[test]
    fn x11_aliases_are_understood_and_utf8_is_preferred() {
        let offered = s(&["STRING", "UTF8_STRING"]);
        let (m, kind) = choose_mime(&offered, &[Accept::Text]).unwrap();
        assert_eq!((m.as_str(), kind), ("UTF8_STRING", PayloadKind::Utf8Text));
        let (m, kind) = choose_mime(&s(&["STRING"]), &[Accept::Text]).unwrap();
        assert_eq!((m.as_str(), kind), ("STRING", PayloadKind::Latin1Text));
        assert!(choose_mime(&s(&["image/png"]), &[Accept::Text, Accept::Files]).is_none());
    }
    #[test]
    fn sources_offer_x11_aliases_and_latin1_for_string() {
        let mut data = DragData::Text("\u{e9}\u{20ac}".into());
        assert_eq!(source_mimes(&data)[2], "UTF8_STRING");
        assert_eq!(&*materialize(&mut data, "STRING").unwrap(), b"\xe9?");
        let utf8 = materialize(&mut data, "text/plain").unwrap();
        assert_eq!(&*utf8, "\u{e9}\u{20ac}".as_bytes());
        let mut files = DragData::Files(vec!["/a b".into()]);
        assert_eq!(source_mimes(&files), FILE_SOURCE_MIMES);
        let list = materialize(&mut files, "text/plain").unwrap();
        assert_eq!(&*list, b"file:///a%20b\r\n");
        let mut lazy = DragData::Lazy {
            mimes: s(&["a/b"]),
            provide: Box::new(|m| Some(m.as_bytes().to_vec())),
        };
        assert_eq!(&*materialize(&mut lazy, "a/b").unwrap(), b"a/b");
        assert!(
            materialize(&mut lazy, "c/d").is_none(),
            "only announced types"
        );
        let mut both = DragData::FilesAndText {
            paths: vec!["/a b".into()],
            text: "/a b".into(),
        };
        assert_eq!(
            source_mimes(&both)[..2],
            ["text/uri-list", "text/plain;charset=utf-8"]
        );
        assert_eq!(
            &*materialize(&mut both, "text/uri-list").unwrap(),
            b"file:///a%20b\r\n"
        );
        assert_eq!(&*materialize(&mut both, "UTF8_STRING").unwrap(), b"/a b");
        assert!(materialize(&mut both, "image/png").is_none());
        assert!(
            materialize(&mut data, "image/png").is_none(),
            "text answers text types only"
        );
        let utf8 = materialize(&mut data, "text/plain;charset=UTF-8").unwrap();
        assert_eq!(
            &*utf8,
            "\u{e9}\u{20ac}".as_bytes(),
            "type names ignore ASCII case"
        );
        assert!(
            materialize(&mut files, "image/png").is_none(),
            "files answer file types only"
        );
    }
}
