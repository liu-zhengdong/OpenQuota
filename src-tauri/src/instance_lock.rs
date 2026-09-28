//! Marks a running OpenQuota application so headless commands can tell whether anything is
//! still refreshing the cached readings.
//!
//! The application holds an exclusive advisory lock on a file next to its database for as long
//! as it runs. The operating system releases the lock when the process exits, including after a
//! crash, so a stale file never reads as a running application.

use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    path::Path,
    thread,
    time::Duration,
};

pub const FILE_NAME: &str = "openquota.instance.lock";

/// A short probe from `openquota pace` may hold the lock for an instant; retrying briefly keeps
/// that probe from costing the application its marker at startup.
const ACQUIRE_ATTEMPTS: u32 = 10;
const ACQUIRE_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Keeps the lock held until dropped.
pub struct InstanceLock {
    _file: File,
}

pub fn acquire(directory: &Path) -> io::Result<InstanceLock> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(FILE_NAME))?;
    let mut attempt = 1;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(InstanceLock { _file: file }),
            Err(TryLockError::WouldBlock) if attempt < ACQUIRE_ATTEMPTS => {
                attempt += 1;
                thread::sleep(ACQUIRE_RETRY_DELAY);
            }
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another process holds the OpenQuota instance lock",
                ))
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// Reports whether an OpenQuota application currently holds the lock in `directory`.
///
/// Returns `None` when the answer cannot be determined. Never creates the lock file.
pub fn is_held(directory: &Path) -> Option<bool> {
    let file = match File::open(directory.join(FILE_NAME)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Some(false),
        Err(_) => return None,
    };
    match file.try_lock_shared() {
        Ok(()) => Some(false),
        Err(TryLockError::WouldBlock) => Some(true),
        Err(TryLockError::Error(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{acquire, is_held, FILE_NAME};

    #[test]
    fn a_held_lock_reads_as_running_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(is_held(directory.path()), Some(false));
        assert!(!directory.path().join(FILE_NAME).exists());

        let lock = acquire(directory.path()).unwrap();
        assert_eq!(is_held(directory.path()), Some(true));
        assert!(acquire(directory.path()).is_err());

        drop(lock);
        assert_eq!(is_held(directory.path()), Some(false));
        assert!(acquire(directory.path()).is_ok());
    }

    #[test]
    fn probing_does_not_block_the_application_from_acquiring() {
        let directory = tempfile::tempdir().unwrap();
        drop(acquire(directory.path()).unwrap());
        for _ in 0..3 {
            assert_eq!(is_held(directory.path()), Some(false));
        }
        let _lock = acquire(directory.path()).unwrap();
        assert_eq!(is_held(directory.path()), Some(true));
    }
}
