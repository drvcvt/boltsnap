//! Exact admission caps tested below the inherited Rust 4096-byte wire envelope.
use super::*;
#[test]
fn exact_component_combined_and_shared_limits_poison_whole_pending_batch() {
    let budget = Budget::default();
    let mut pending = Pending::default();
    pending.commit(Some("c".repeat(4096)), &budget);
    pending.preedit(Some("p".repeat(4096)), -1, -1, &budget);
    assert_eq!(pending.poisoned(), None);
    assert_eq!(budget.used(), 8192);
    let transaction = pending.take();
    assert_eq!(transaction.commit.as_ref().unwrap().as_str().len(), 4096);
    assert_eq!(
        transaction.preedit.as_ref().unwrap().text.as_str().len(),
        4096
    );
    let mut held = vec![transaction];
    // One over the cap poisons the batch and returns the credits of the accepted commit.
    pending.commit(Some("c".repeat(4096)), &budget);
    assert_eq!(budget.used(), 8192 + 4096);
    pending.preedit(Some("x".repeat(4097)), -1, -1, &budget);
    assert!(pending.poisoned().is_some());
    assert_eq!(budget.used(), 8192);
    // A poisoned batch ignores later components and stays poisoned until taken.
    pending.commit(Some("late".into()), &budget);
    pending.delete(1, 1);
    assert_eq!(budget.used(), 8192);
    assert_eq!(pending.take(), ImeTransaction::default());
    assert_eq!(pending.poisoned(), None);
    for _ in 0..15 {
        pending.commit(Some("c".repeat(4096)), &budget);
        pending.preedit(Some("p".repeat(4096)), -1, -1, &budget);
        held.push(pending.take());
    }
    assert_eq!(budget.used(), 128 * 1024);
    pending.commit(Some("x".into()), &budget);
    assert!(pending.poisoned().is_some());
    assert_eq!(pending.take(), ImeTransaction::default());
    let escaped = held[0].commit.as_ref().unwrap().shared();
    drop(held);
    assert_eq!(budget.used(), 4096);
    drop(escaped);
    assert_eq!(budget.used(), 0);
}
#[test]
fn nul_and_cursor_violations_poison_with_distinct_reasons() {
    let budget = Budget::default();
    let mut pending = Pending::default();
    pending.commit(Some("a\0b".into()), &budget);
    assert_eq!(pending.poisoned(), Some("NUL in IME text"));
    pending.take();
    for (text, begin, end) in [
        (Some("é"), 1, 2),
        (Some("é"), -1, 0),
        (Some("é"), 0, 99),
        (None, 1, 1),
    ] {
        pending.commit(Some("kept".into()), &budget);
        pending.preedit(text.map(Into::into), begin, end, &budget);
        assert!(pending.poisoned().is_some());
        assert_eq!(budget.used(), 0);
        assert_eq!(pending.take(), ImeTransaction::default());
    }
}
