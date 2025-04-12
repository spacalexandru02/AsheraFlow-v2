// Modified src/validators/path_validator.rs
use std::path::Path;
use crate::errors::error::Error;

pub struct PathValidator;

impl PathValidator {
    // For init command we need to be more flexible - allow creating new directories
    pub fn validate_for_init(path: &str) -> Result<(), Error> {
        if path.is_empty() {
            return Err(Error::InvalidPath("Path cannot be empty".to_string()));
        }

        // For init, we just need to check if the parent directory exists
        let path_obj = Path::new(path);
        
        // If the path already exists and is a directory, that's fine
        if path_obj.exists() {
            if !path_obj.is_dir() {
                return Err(Error::InvalidPath(format!("'{}' exists but is not a directory", path_obj.display())));
            }
            return Ok(());
        }
        
        // If it doesn't exist, check if the parent exists
        if let Some(parent) = path_obj.parent() {
            if !parent.exists() {
                return Err(Error::InvalidPath(format!("Parent directory '{}' does not exist", parent.display())));
            }
            if !parent.is_dir() {
                return Err(Error::InvalidPath(format!("'{}' is not a directory", parent.display())));
            }
        }
        
        Ok(())
    }

    // Original validation for other commands
    pub fn validate(path: &str) -> Result<(), Error> {
        if path.is_empty() {
            return Err(Error::InvalidPath("Path cannot be empty".to_string()));
        }

        let path = Path::new(path);
        if !path.exists() {
            return Err(Error::InvalidPath(format!("Path '{}' does not exist", path.display())));
        }

        if !path.is_dir() {
            return Err(Error::InvalidPath(format!("'{}' is not a directory", path.display())));
        }

        Ok(())
    }
}