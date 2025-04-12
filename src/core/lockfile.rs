use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum LockError {
    MissingParent(String),
    NoPermission(String),
    StaleLock(String),
    LockDenied(String),
}

pub struct Lockfile {
    file_path: PathBuf,
    lock_path: PathBuf,
    lock: Option<File>,
}

impl Lockfile {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let file_path = path.as_ref().to_path_buf();
        let lock_path = file_path.with_extension("lock");
        Lockfile {
            file_path,
            lock_path,
            lock: None,
        }
    }

    pub fn hold_for_update(&mut self) -> Result<bool, LockError> {
        if self.lock.is_some() {
            return Ok(true);
        }

        let dir = self.lock_path.parent().ok_or_else(|| {
            LockError::MissingParent(format!("Parent directory does not exist: {:?}", self.lock_path))
        })?;

        // Create parent directory if it doesn't exist
        fs::create_dir_all(dir).map_err(|e| match e.kind() {
            io::ErrorKind::PermissionDenied => LockError::NoPermission(format!(
                "Cannot create directory '{}': Permission denied", dir.display()
            )),
            _ => LockError::MissingParent(format!(
                "Failed to create directory '{}': {}", dir.display(), e
            )),
        })?;

        // Try to create the lock file
        match OpenOptions::new()
            .write(true)
            .create_new(true) // O_CREAT | O_EXCL
            .open(&self.lock_path)
        {
            Ok(file) => {
                self.lock = Some(file);
                Ok(true)
            }
            Err(e) => match e.kind() {
                io::ErrorKind::AlreadyExists => Err(LockError::LockDenied(format!(
                    "Unable to create '{}': File exists.\nAnother process seems to be running in this repository.\n\
                     If it still fails, a process may have crashed in this repository earlier:\n\
                     remove the file manually to continue.", self.lock_path.display()
                ))),
                io::ErrorKind::PermissionDenied => Err(LockError::NoPermission(format!(
                    "Permission denied when creating lock file '{}'", self.lock_path.display()
                ))),
                _ => Err(LockError::MissingParent(format!(
                    "Failed to create lock file '{}': {}", self.lock_path.display(), e
                ))),
            },
        }
    }

    pub fn write(&mut self, data: &str) -> Result<(), LockError> {
        let lock = self.lock.as_mut().ok_or_else(|| {
            LockError::StaleLock(format!(
                "Not holding lock on file '{}'", self.file_path.display()
            ))
        })?;
        
        lock.write_all(data.as_bytes())
            .map_err(|e| LockError::StaleLock(format!(
                "Failed to write to lock file '{}': {}", self.lock_path.display(), e
            )))?;
        Ok(())
    }

    // Close and remove the lock file
    pub fn rollback(&mut self) -> Result<(), LockError> {
        if self.lock.is_none() {
            return Ok(());  // No lock to release
        }
        
        // Drop the file handle
        self.lock.take();
        
        // Remove the lock file
        match fs::remove_file(&self.lock_path) {
            Ok(_) => Ok(()),
            Err(e) => match e.kind() {
                io::ErrorKind::NotFound => Ok(()), // File is already gone, that's fine
                _ => Err(LockError::StaleLock(format!(
                    "Failed to remove lock file '{}': {}", self.lock_path.display(), e
                ))),
            },
        }
    }

    // Modified to take a reference to self and not consume it
    pub fn commit_ref(&mut self) -> Result<(), LockError> {
        let lock = self.lock.take().ok_or_else(|| {
            LockError::StaleLock(format!(
                "Not holding lock on file '{}'", self.file_path.display()
            ))
        })?;

        // Close the file before rename (necessary on Windows)
        drop(lock);
        
        fs::rename(&self.lock_path, &self.file_path)
            .map_err(|e| LockError::StaleLock(format!(
                "Failed to rename lock file '{}' to '{}': {}", 
                self.lock_path.display(), self.file_path.display(), e
            )))?;
        
        Ok(())
    }

    pub fn write_bytes(&mut self, data: &[u8]) -> Result<(), LockError> {
        let lock = self.lock.as_mut().ok_or_else(|| {
            LockError::StaleLock(format!(
                "Not holding lock on file '{}'", self.file_path.display()
            ))
        })?;
        
        lock.write_all(data)
            .map_err(|e| LockError::StaleLock(format!(
                "Failed to write to lock file '{}': {}", self.lock_path.display(), e
            )))?;
        Ok(())
    }
}