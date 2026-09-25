use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static PART_ID: AtomicU64 = AtomicU64::new(0);

/// Publish a complete, synced copy without replacing an existing destination.
pub(crate) fn copy_noreplace(input: &mut impl Read, destination: &Path) -> io::Result<()> {
    let (part, mut output) = create_part(destination)?;
    let result = (|| {
        io::copy(input, &mut output)?;
        output.sync_all()?;
        rename_noreplace(&part, destination)
    })();
    drop(output);
    if result.is_err() {
        let _ = fs::remove_file(part);
    }
    result
}

fn create_part(destination: &Path) -> io::Result<(PathBuf, File)> {
    loop {
        let mut name = destination.as_os_str().to_os_string();
        name.push(format!(
            ".part-{}-{}",
            std::process::id(),
            PART_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let part = PathBuf::from(name);
        match OpenOptions::new().write(true).create_new(true).open(&part) {
            Ok(output) => return Ok((part, output)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    let source_c = std::ffi::CString::new(source.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination_c = std::ffi::CString::new(destination.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source_c.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(libc::ENOSYS) {
        return Err(error);
    }

    fs::hard_link(source, destination)?;
    if let Err(error) = fs::remove_file(source) {
        if let Err(cleanup) = fs::remove_file(destination) {
            eprintln!(
                "boltsnap: failed to roll back no-replace link {}: {cleanup}",
                destination.display()
            );
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::sync::Barrier;

    fn temp_dir() -> PathBuf {
        loop {
            let dir = std::env::temp_dir().join(format!(
                "boltsnap-files-{}-{}",
                std::process::id(),
                PART_ID.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => return dir,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory: {error}"),
            }
        }
    }

    struct GatedReader<'a> {
        input: File,
        ready: Option<&'a Barrier>,
    }

    impl Read for GatedReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if let Some(ready) = self.ready.take() {
                ready.wait();
            }
            self.input.read(buffer)
        }
    }

    #[test]
    fn concurrent_copies_publish_one_complete_file_and_preserve_sources() {
        let dir = temp_dir();
        let destination = dir.join("saved.mp4");
        let sources = [dir.join("first.mp4"), dir.join("second.mp4")];
        let contents = [vec![1; 128 * 1024], vec![2; 128 * 1024]];
        for (source, content) in sources.iter().zip(&contents) {
            fs::write(source, content).unwrap();
        }
        let ready = Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let jobs: Vec<_> = sources
                .iter()
                .map(|source| {
                    let ready = &ready;
                    let destination = &destination;
                    scope.spawn(move || {
                        let mut reader = GatedReader {
                            input: File::open(source).unwrap(),
                            ready: Some(ready),
                        };
                        let result = copy_noreplace(&mut reader, destination);
                        // Also release the other reader if part creation failed.
                        if let Some(ready) = reader.ready.take() {
                            ready.wait();
                        }
                        result
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let winner = results.iter().position(Result::is_ok).unwrap();
        assert_eq!(
            results[1 - winner].as_ref().unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(destination).unwrap(), contents[winner]);
        for (source, content) in sources.iter().zip(&contents) {
            assert_eq!(&fs::read(source).unwrap(), content);
        }
        assert_eq!(fs::read_dir(dir).unwrap().count(), 3);
    }

    #[test]
    fn failed_read_removes_only_its_own_partial_copy() {
        struct FailedRead;
        impl Read for FailedRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected read failure"))
            }
        }

        let dir = temp_dir();
        let destination = dir.join("saved.mp4");
        let foreign = dir.join("saved.mp4.part-foreign");
        fs::write(&foreign, b"another save").unwrap();
        let mut input = b"partial".as_slice().chain(FailedRead);
        let error = copy_noreplace(&mut input, &destination).unwrap_err();
        assert_eq!(error.to_string(), "injected read failure");
        assert!(!destination.exists());
        assert_eq!(fs::read(foreign).unwrap(), b"another save");
        assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    }

    #[test]
    fn copy_preserves_non_utf8_destination_paths() {
        let dir = temp_dir().join(OsString::from_vec(b"directory-\xff".to_vec()));
        fs::create_dir(&dir).unwrap();
        let destination = dir.join(OsString::from_vec(b"video-\xfe.mp4".to_vec()));
        copy_noreplace(&mut b"video".as_slice(), &destination).unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"video");
        assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    }
}
