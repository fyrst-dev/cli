//! Exclusive lock for overlapping backup runs (`var/backup-runtime.lock`).

use super::error::Error;
use std::fs::{self, File, OpenOptions};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

/// Holds an exclusive flock on `compose_dir/var/backup-runtime.lock`.
/// Released when dropped (fd close).
#[derive(Debug)]
pub struct BackupLock {
    _file: File,
}

pub fn acquire(compose_dir: &Path) -> Result<BackupLock, Error> {
    let lock_path = compose_dir.join("var/backup-runtime.lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            Error::fail(format!("cannot create {}: {e}", parent.display()))
        })?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| Error::fail(format!("cannot open {}: {e}", lock_path.display())))?;

    #[cfg(unix)]
    {
        if !try_flock_exclusive_nb(&file) {
            return Err(Error::fail(format!(
                "Another fyrst-cli shopware backup run holds {}. Cron overlap — wait or remove a stale lock.",
                lock_path.display()
            )));
        }
    }

    Ok(BackupLock { _file: file })
}

#[cfg(unix)]
fn try_flock_exclusive_nb(file: &File) -> bool {
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "fyrst-cli-lock-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn second_acquire_is_refused() {
        let dir = temp_dir("overlap");
        let first = acquire(&dir).unwrap();
        let err = acquire(&dir).unwrap_err();
        assert!(err.to_string().contains("holds"), "{err}");
        assert!(err.to_string().contains("backup-runtime.lock"), "{err}");
        drop(first);
        acquire(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
