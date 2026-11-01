//! Literal accounting for aliases, accepted sends and reserved queued outcomes.
use super::*;

#[test]
fn aliases_and_jobs_share_one_charge_until_last_host_holder_drops() {
    let mut budget = Budget::default();
    let bytes: Arc<[u8]> = vec![7; 32].into();
    let source = budget.payload(&bytes).unwrap();
    let alias = budget.payload(&bytes).unwrap();
    let job = source.clone();
    assert_eq!(budget.used.load(Ordering::Acquire), 32);
    drop(source);
    drop(alias);
    assert_eq!(budget.used.load(Ordering::Acquire), 32);
    assert_eq!(job.bytes().as_ref(), &[7; 32]);
    drop(job);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
    // Application ownership of bytes does not hold host credit after collection.
    assert_eq!(bytes.as_ref(), &[7; 32]);
}

#[test]
fn exact_shared_limit_rejection_preserves_existing_credit() {
    let budget = Budget::default();
    let queued = budget.reserve(LIMIT - 1).unwrap();
    let read = budget.reserve(1).unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), LIMIT);
    assert!(matches!(budget.reserve(1), Err(Error::LimitExceeded)));
    assert!(matches!(
        budget.reserve(usize::MAX),
        Err(Error::LimitExceeded)
    ));
    assert_eq!(budget.used.load(Ordering::Acquire), LIMIT);
    drop(read);
    assert_eq!(budget.used.load(Ordering::Acquire), LIMIT - 1);
    drop(queued);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn equal_bytes_with_distinct_allocations_have_distinct_storage_charges() {
    let mut budget = Budget::default();
    let a = budget.payload(&Arc::<[u8]>::from([1, 2])).unwrap();
    let b = budget.payload(&Arc::<[u8]>::from([1, 2])).unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 4);
    drop(a);
    assert_eq!(budget.used.load(Ordering::Acquire), 2);
    drop(b);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}
