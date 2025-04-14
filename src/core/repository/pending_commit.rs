use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{fs as fs_std, io};

use crate::errors::error::Error;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum PendingCommitType {
    Merge,
    CherryPick,
    Revert,
}

#[derive(Debug)]
pub struct PendingCommit {
    pathname: PathBuf,
    pub message_path: PathBuf,
}

impl PendingCommit {
    pub fn new(pathname: &Path) -> Self {
        Self {
            pathname: pathname.to_owned(),
            message_path: pathname.join("MERGE_MSG"),
        }
    }

    pub fn start(&self, oid: &str, r#type: PendingCommitType) -> Result<(), Error> {
        let path = match r#type {
            PendingCommitType::Merge => self.pathname.join("MERGE_HEAD"),
            PendingCommitType::CherryPick => self.pathname.join("CHERRY_PICK_HEAD"),
            PendingCommitType::Revert => self.pathname.join("REVERT_HEAD"),
        };
        
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| Error::Generic(format!("Failed to create merge head file: {}", e)))?
            .write_all(oid.as_bytes())
            .map_err(|e| Error::Generic(format!("Failed to write to merge head file: {}", e)))?;

        Ok(())
    }

    pub fn in_progress(&self, r#type: PendingCommitType) -> bool {
        match r#type {
            PendingCommitType::Merge => self.pathname.join("MERGE_HEAD").exists(),
            PendingCommitType::CherryPick => self.pathname.join("CHERRY_PICK_HEAD").exists(),
            PendingCommitType::Revert => self.pathname.join("REVERT_HEAD").exists(),
        }
    }

    pub fn merge_type(&self) -> Option<PendingCommitType> {
        if self.pathname.join("MERGE_HEAD").exists() {
            return Some(PendingCommitType::Merge);
        } else if self.pathname.join("CHERRY_PICK_HEAD").exists() {
            return Some(PendingCommitType::CherryPick);
        } else if self.pathname.join("REVERT_HEAD").exists() {
            return Some(PendingCommitType::Revert);
        }
        None
    }

    pub fn merge_oid(&self, r#type: PendingCommitType) -> Result<String, Error> {
        let head_path = match r#type {
            PendingCommitType::Merge => self.pathname.join("MERGE_HEAD"),
            PendingCommitType::CherryPick => self.pathname.join("CHERRY_PICK_HEAD"),
            PendingCommitType::Revert => self.pathname.join("REVERT_HEAD"),
        };

        match fs::read_to_string(&head_path) {
            Ok(oid) => Ok(oid.trim().to_string()),
            Err(err) => {
                if err.kind() == io::ErrorKind::NotFound {
                    let name = head_path.file_name().unwrap().to_string_lossy().to_string();

                    Err(Error::Generic(format!("No {} in progress", name)))
                } else {
                    Err(Error::Generic(format!("Failed to read merge head file: {}", err)))
                }
            }
        }
    }

    pub fn merge_message(&self) -> Result<String, Error> {
        match fs::read_to_string(&self.message_path) {
            Ok(message) => Ok(message),
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    // If the merge message file doesn't exist, return an empty string
                    Ok(String::new())
                } else {
                    Err(Error::Generic(format!("Failed to read merge message: {}", e)))
                }
            }
        }
    }

    pub fn clear(&self, r#type: PendingCommitType) -> Result<(), Error> {
        let head_path = match r#type {
            PendingCommitType::Merge => self.pathname.join("MERGE_HEAD"),
            PendingCommitType::CherryPick => self.pathname.join("CHERRY_PICK_HEAD"),
            PendingCommitType::Revert => self.pathname.join("REVERT_HEAD"),
        };

        match fs::remove_file(&head_path) {
            Ok(()) => (),
            Err(err) => return self.handle_no_merge_to_abort(&head_path, err),
        }
        
        // Also remove the message file if it exists
        if self.message_path.exists() {
            fs::remove_file(&self.message_path)
                .map_err(|e| Error::Generic(format!("Failed to remove merge message file: {}", e)))?;
        }

        Ok(())
    }

    fn handle_no_merge_to_abort(&self, head_path: &Path, err: io::Error) -> Result<(), Error> {
        if err.kind() == io::ErrorKind::NotFound {
            let name = head_path.file_name().unwrap().to_string_lossy().to_string();
            Err(Error::Generic(format!("No merge to abort ({})", name)))
        } else {
            Err(Error::Generic(format!("Failed to remove merge head file: {}", err)))
        }
    }
}
