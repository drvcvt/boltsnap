//! L3 and R3: what fills the shelf's 2 GiB quota. A long recording heads for
//! `record_dir` and a dead daemon's segments wait for recovery; neither
//! blocks clips, imports or a replay restart.

use super::*;

fn file(dir: &Path, name: &str, len: u64) {
    File::create(dir.join(name)).unwrap().set_len(len).unwrap();
}

#[test]
fn segments_of_a_dead_daemon_never_fill_the_quota() {
    let dir = temp_dir("cache-orphan");
    let dead = std::process::id().wrapping_add(1);
    let segment = format!("boltsnap-segment-{dead}-0-DP_3.mkv");
    file(&dir, &segment, RECORDING_CACHE_LIMIT_BYTES);
    assert_eq!(check_recording_cache_limit(&dir), Ok(()));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn own_segments_count_until_the_recording_heads_for_disk() {
    let dir = temp_dir("cache-own");
    let own = std::process::id();
    file(&dir, &format!("boltsnap-segment-{own}-3-DP_3.mkv"), 1000);
    file(
        &dir,
        &format!("boltsnap-segment-{own}-3-DP_3.cursor.json"),
        20,
    );
    file(&dir, "boltsnap-2026-11-01_12-00-00.mp4", 5);
    assert_eq!(cache_bytes(&dir, own, false), Ok(1025));
    assert_eq!(cache_bytes(&dir, own, true), Ok(5));
    let _ = fs::remove_dir_all(dir);
}
