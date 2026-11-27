use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::StorageError;
use crate::model::Sanitizable;

static TMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Thread-safe, atomic JSON configuration store backed by a file on disk.
#[derive(Debug)]
pub struct ConfigStore<T> {
    path: PathBuf,
    state: Mutex<T>,
}

impl<T> ConfigStore<T> {
    /// Returns the filesystem path associated with this configuration store.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl<T> ConfigStore<T>
where
    T: Default + Clone + Serialize + DeserializeOwned + Sanitizable,
{
    /// Loads configuration from `path`.
    ///
    /// - If `path` does not exist, initializes sanitized `T::default()` in memory
    ///   (parent directories are created lazily on `save` / `update`).
    /// - If `path` contains malformed JSON, renames the corrupt file to
    ///   `<path>.corrupt.<ts>` and resets cleanly to sanitized `T::default()`.
    pub fn new(path: PathBuf) -> Result<Self, StorageError> {
        let mut value = match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<T>(&contents) {
                Ok(parsed) => parsed,
                Err(_) => {
                    backup_corrupt_file(&path)?;
                    T::default()
                }
            },
            Err(err) if err.kind() == ErrorKind::NotFound => T::default(),
            Err(err) => return Err(StorageError::Io { path, source: err }),
        };

        value.sanitize();
        Ok(Self {
            path,
            state: Mutex::new(value),
        })
    }

    /// Returns a cloned snapshot of the current in-memory configuration.
    pub fn get(&self) -> T {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Mutates the configuration via `f`, sanitizes it, atomically writes it to disk,
    /// and updates the in-memory state if the disk write succeeds.
    pub fn update<F>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce(&mut T),
    {
        let mut guard = self.state.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut next = guard.clone();
        f(&mut next);
        next.sanitize();
        atomic_write_json(&self.path, &next)?;
        *guard = next.clone();
        Ok(next)
    }

    /// Replaces the configuration with `value`, sanitizes it, atomically writes it to disk,
    /// and commits it to the in-memory state.
    pub fn save(&self, mut value: T) -> Result<T, StorageError> {
        let mut guard = self.state.lock().map_err(|_| StorageError::LockPoisoned)?;
        value.sanitize();
        atomic_write_json(&self.path, &value)?;
        *guard = value.clone();
        Ok(value)
    }
}

/// Renames a corrupt file at `path` to `<path>.corrupt.<timestamp_ms>` (disambiguating collisions).
pub(crate) fn backup_corrupt_file(path: &Path) -> Result<PathBuf, StorageError> {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut suffix = 0u32;
    let backup_path = loop {
        let mut os = OsString::from(path.as_os_str());
        if suffix == 0 {
            os.push(format!(".corrupt.{ts_ms}"));
        } else {
            os.push(format!(".corrupt.{ts_ms}.{suffix}"));
        }
        let candidate = PathBuf::from(os);
        if !candidate.exists() {
            break candidate;
        }
        suffix = suffix.saturating_add(1);
    };

    if std::fs::rename(path, &backup_path).is_err() {
        std::fs::copy(path, &backup_path).map_err(|source| StorageError::Io {
            path: backup_path.clone(),
            source,
        })?;
        std::fs::remove_file(path).map_err(|source| StorageError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }

    Ok(backup_path)
}

/// Atomically writes `value` as pretty-printed JSON to `path` via a same-directory temp file,
/// `sync_all()`, explicit handle drop, and atomic rename.
pub(crate) fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| StorageError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }

    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');

    let seq = TMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut tmp_os = OsString::from(path.as_os_str());
    tmp_os.push(format!(".tmp.{pid}.{seq}"));
    let tmp_path = PathBuf::from(tmp_os);

    let write_res = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(source) = write_res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(StorageError::Io {
            path: tmp_path,
            source,
        });
    }

    let mut attempts = 0u8;
    loop {
        match std::fs::rename(&tmp_path, path) {
            Ok(()) => return Ok(()),
            Err(err) if attempts < 2 && is_transient_windows_rename_error(&err) => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(source) => {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(StorageError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }
}

fn is_transient_windows_rename_error(err: &std::io::Error) -> bool {
    if err.kind() == ErrorKind::PermissionDenied {
        return true;
    }
    matches!(err.raw_os_error(), Some(5) | Some(32))
}
