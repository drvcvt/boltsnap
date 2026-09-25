use std::fs::{self, File};
use std::io::{self, Seek};
use std::path::{Path, PathBuf};

/// Copies an image card to a fresh `boltsnap-<local time>[-N].png` in `dir`.
/// Existing files and symlinks are never replaced; a name taken between the
/// pick and the publish moves on to the next suffix.
pub(super) fn save_image(source: &Path, dir: &Path) -> io::Result<PathBuf> {
    save_image_at(source, dir, &crate::paths::local_timestamp())
}

fn save_image_at(source: &Path, dir: &Path, stamp: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut input = File::open(source)?;
    loop {
        let destination = crate::paths::unique_saved_path_at(dir, None, stamp, "png");
        match crate::platform::files::copy_noreplace(&mut input, &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => input.rewind()?,
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAMP: &str = "2026-09-25_12-00-00";

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "boltsnap-save-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("out")).unwrap();
        dir
    }

    #[test]
    fn two_saves_in_the_same_second_keep_both_images() {
        let dir = test_dir("same-second");
        let out = dir.join("out");
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        fs::write(&a, b"first").unwrap();
        fs::write(&b, b"second").unwrap();

        let first = save_image_at(&a, &out, STAMP).unwrap();
        let second = save_image_at(&b, &out, STAMP).unwrap();

        assert_eq!(first, out.join(format!("boltsnap-{STAMP}.png")));
        assert_eq!(second, out.join(format!("boltsnap-{STAMP}-1.png")));
        assert_eq!(fs::read(first).unwrap(), b"first");
        assert_eq!(fs::read(second).unwrap(), b"second");
        assert_eq!(fs::read(a).unwrap(), b"first");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_dangling_symlink_keeps_its_name() {
        let dir = test_dir("symlink");
        let out = dir.join("out");
        let source = dir.join("shot.png");
        fs::write(&source, b"image").unwrap();
        let taken = out.join(format!("boltsnap-{STAMP}.png"));
        let target = dir.join("elsewhere.png");
        std::os::unix::fs::symlink(&target, &taken).unwrap();

        let saved = save_image_at(&source, &out, STAMP).unwrap();

        assert_eq!(saved, out.join(format!("boltsnap-{STAMP}-1.png")));
        assert_eq!(fs::read_link(&taken).unwrap(), target);
        assert!(!target.exists());
        assert_eq!(fs::read(saved).unwrap(), b"image");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_source_saves_nothing() {
        let dir = test_dir("missing");
        let out = dir.join("out");
        assert!(save_image_at(&dir.join("gone.png"), &out, STAMP).is_err());
        assert_eq!(fs::read_dir(out).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}
