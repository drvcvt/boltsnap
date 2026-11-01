//! Exact eager outputs, owner reuse, admission overlap and Lazy lifetime accounting.
use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn aliases_share_the_original_buffer_and_last_send_keeps_it_alive() {
    let text = "hello".repeat(100);
    let pointer = text.as_ptr();
    let mut data = Data::new(DragData::Text(text), 64, Some(OutgoingLimits::default())).unwrap();
    let first = data.send("UTF8_STRING").unwrap();
    assert_eq!(pointer, first.as_ptr());
    for mime in interop::TEXT_SOURCE_MIMES {
        assert!(Arc::ptr_eq(&first.0, &data.send(mime).unwrap().0));
    }
    let weak = Arc::downgrade(&first.0);
    drop(data);
    assert_eq!(weak.strong_count(), 1);
    drop(first);
    assert!(weak.upgrade().is_none());
}

fn compare(mut original: DragData, data: DragData) {
    let mimes = interop::source_mimes(&original);
    let mut data = Data::new(data, 64, None).unwrap();
    assert_eq!(data.mimes(), mimes);
    for mime in mimes
        .iter()
        .map(String::as_str)
        .chain(["image/unknown", "text/plain;charset=UTF-8"])
    {
        let expected = interop::materialize(&mut original, mime);
        let actual = data.send(mime);
        assert_eq!(actual.as_deref(), expected.as_deref());
    }
}
#[test]
fn eager_outputs_match_original_alias_and_duplicate_rules() {
    for text in ["", "ASCII", "é界 a\n\0", "e\u{301}🙂"] {
        compare(DragData::Text(text.into()), DragData::Text(text.into()));
        let both = || DragData::FilesAndText {
            paths: vec![PathBuf::from("/a b%")],
            text: text.into(),
        };
        compare(both(), both());
    }
    let paths = || vec![PathBuf::from("/a b%"), PathBuf::from("/é界?!$'()*,:@")];
    compare(DragData::Files(paths()), DragData::Files(paths()));
    let fixed = || {
        DragData::Static(vec![
            ("a/b".into(), b"one".to_vec()),
            ("a/b".into(), b"two".to_vec()),
            ("c/d".into(), b"one".to_vec()),
        ])
    };
    let mut data = Data::new(fixed(), 64, None).unwrap();
    assert!(Arc::ptr_eq(
        &data.send("a/b").unwrap().0,
        &data.send("c/d").unwrap().0
    ));
    compare(fixed(), fixed());
}

#[test]
fn whole_payload_and_temporary_source_overlap_are_admitted_first() {
    let policy = OutgoingLimits {
        max_payload: 8,
        max_eager_bytes: 16,
    };
    assert!(Data::new(DragData::Text("12345678".into()), 64, Some(policy)).is_ok());
    assert!(matches!(
        Data::new(DragData::Text("123456789".into()), 64, Some(policy)),
        Err(Error::LimitExceeded)
    ));
    let mut text = String::with_capacity(17);
    text.push('x');
    assert!(matches!(
        Data::new(DragData::Text(text), 64, Some(policy)),
        Err(Error::LimitExceeded)
    ));
    let policy = OutgoingLimits {
        max_payload: 20,
        max_eager_bytes: 16,
    };
    // The 15-byte representation fits, but its preparation overlaps the four source bytes.
    assert!(matches!(
        Data::new(
            DragData::Files(vec![PathBuf::from("/a b")]),
            64,
            Some(policy)
        ),
        Err(Error::LimitExceeded)
    ));
    assert!(Data::new(DragData::Files(vec![PathBuf::from("/a b")]), 64, None).is_ok());
}

#[test]
fn lazy_calls_stay_stateful_and_live_capacity_follows_last_pipe_owner() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let raw = DragData::Lazy {
        mimes: vec!["a/b".into()],
        provide: Box::new(move |_| Some(vec![counter.fetch_add(1, Ordering::Relaxed) as u8; 4])),
    };
    let mut data = Data::new(
        raw,
        64,
        Some(OutgoingLimits {
            max_payload: 4,
            max_eager_bytes: 8,
        }),
    )
    .unwrap();
    assert!(data.send("unknown").is_none());
    let first = data.send("a/b").unwrap();
    let second = data.send("a/b").unwrap();
    assert_eq!(&*first, &[0; 4]);
    assert_eq!(&*second, &[1; 4]);
    assert!(data.send("a/b").is_none());
    assert_eq!(calls.load(Ordering::Relaxed), 3);
    drop(first);
    assert_eq!(&*data.send("a/b").unwrap(), &[3; 4]);
    drop(second);
    assert_eq!(&*data.send("a/b").unwrap(), &[4; 4]);
}
