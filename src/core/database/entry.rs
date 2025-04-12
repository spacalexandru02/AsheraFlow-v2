use std::path::PathBuf;

use crate::core::file_mode::FileMode;

#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseEntry {
    pub name: String,
    pub oid: String,
    pub mode: String, // We still store mode as string for serialization compatibility
}

impl DatabaseEntry {
    pub fn new(name: String, oid: String, mode: &str) -> Self {
        // Standardize mode using FileMode
        let file_mode = FileMode::parse(mode);
        
        DatabaseEntry {
            name,
            oid,
            mode: file_mode.to_octal_string(),
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_oid(&self) -> &str {
        &self.oid
    }
    
    pub fn get_mode(&self) -> &str {
        &self.mode
    }
    
    // Helper method to get the FileMode object
    pub fn get_file_mode(&self) -> FileMode {
        FileMode::parse(&self.mode)
    }
    
    pub fn parent_directories(&self) -> Vec<PathBuf> {
        let path = PathBuf::from(&self.name);
        let mut dirs = Vec::new();
        
        let mut current = path.clone();
        while let Some(parent) = current.parent() {
            if !parent.as_os_str().is_empty() {
                dirs.push(parent.to_path_buf());
            }
            current = parent.to_path_buf();
        }
        
        // Reverse to get them in ascending order
        dirs.reverse();
        dirs
    }
    
    pub fn basename(&self) -> String {
        let path = PathBuf::from(&self.name);
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}