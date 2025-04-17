// src/core/refs.rs
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use regex::Regex;
use crate::errors::error::Error;
use crate::core::lockfile::Lockfile;

// Constants
pub const HEAD: &str = "HEAD";
const DEFAULT_BRANCH: &str = "master";
const SYMREF_PREFIX: &str = "ref: ";
lazy_static::lazy_static! {
    static ref SYMREF_REGEX: Regex = Regex::new(r"^ref: (.+)$").unwrap();
}

// Reference types
#[derive(Debug, Clone, PartialEq)]
pub enum Reference {
    Direct(String),       // Direct reference to an OID
    Symbolic(String),     // Symbolic reference to another ref
}

// Custom errors
#[derive(Debug)]
pub enum RefError {
    InvalidBranch(String),
    LockFailed(String),
}

pub struct Refs {
    pathname: PathBuf,
    refs_path: PathBuf,
    heads_path: PathBuf,
}

impl Refs {
    pub fn new<P: AsRef<Path>>(pathname: P) -> Self {
        let path = pathname.as_ref().to_path_buf();
        let refs_path = path.join("refs");
        let heads_path = refs_path.join("heads");
        
        Refs {
            pathname: path,
            refs_path,
            heads_path,
        }
    }

    // Read HEAD reference, following symbolic references
    pub fn read_head(&self) -> Result<Option<String>, Error> {
        let head_path = self.pathname.join(HEAD);
        if !head_path.exists() {
            return Ok(None);
        }
        
        self.read_symref(&head_path)
    }

    // Set HEAD to point to a branch or commit
    pub fn set_head(&self, revision: &str, oid: &str) -> Result<(), Error> {
        let head_path = self.pathname.join(HEAD);
        let branch_path = self.heads_path.join(revision);
        
        if File::open(&branch_path).is_ok() {
            // If the revision is a valid branch name, create a symbolic ref
            let relative = branch_path.strip_prefix(&self.pathname)
                .map_err(|_| Error::PathResolution(format!(
                    "Failed to create relative path from '{}' to '{}'",
                    self.pathname.display(), branch_path.display()
                )))?;
                
            self.update_ref_file(&head_path, &format!("{}{}", SYMREF_PREFIX, relative.display()))
        } else {
            // Otherwise, store the commit ID directly
            self.update_ref_file(&head_path, oid)
        }
    }

    // Update HEAD, following symbolic references
    pub fn update_head(&self, oid: &str) -> Result<(), Error> {
        self.update_symref(&self.pathname.join(HEAD), oid)
    }
    
    // Update a reference directly with an OID
    pub fn update_ref(&self, name: &str, oid: &str) -> Result<(), Error> {
        // Determine correct path for the reference
        let ref_path = if name.starts_with("refs/") {
            self.pathname.join(name)
        } else {
            self.refs_path.join(name)
        };
        
        // Create parent directories if they don't exist
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                Error::DirectoryCreation(format!(
                    "Failed to create directory '{}': {}",
                    parent.display(),
                    e
                ))
            })?;
        }
        
        // Update the reference file
        self.update_ref_file(&ref_path, oid)
    }
    
    // Create a new branch pointing to the specified commit OID
    pub fn create_branch(&self, branch_name: &str, oid: &str) -> Result<(), Error> {
        // Validate branch name using regex pattern for invalid names
        if !self.is_valid_branch_name(branch_name) {
            return Err(Error::Generic(format!(
                "'{}' is not a valid branch name.", branch_name
            )));
        }
        
        // Check if branch already exists
        let branch_path = self.heads_path.join(branch_name);
        if branch_path.exists() {
            return Err(Error::Generic(format!(
                "A branch named '{}' already exists.", branch_name
            )));
        }
        
        // Create the branch reference file
        self.update_ref_file(&branch_path, oid)
    }
    
    // Read a reference by name (branch, HEAD, etc.)
    pub fn read_ref(&self, name: &str) -> Result<Option<String>, Error> {
        // Check for HEAD alias
        if name == "@" || name == HEAD {
            return self.read_head();
        }
        
        // Look in multiple locations in order:
        // 1. Direct under .ash directory
        // 2. Under .ash/refs
        // 3. Under .ash/refs/heads (branches)
        let paths = [
            self.pathname.join(name),
            self.refs_path.join(name),
            self.heads_path.join(name),
        ];
        
        for path in &paths {
            if path.exists() {
                return self.read_symref(path);
            }
        }
        
        // Reference not found
        Ok(None)
    }
    
    // Read a reference file and parse as OID or symref
    fn read_oid_or_symref(&self, path: &Path) -> Result<Option<Reference>, Error> {
        if !path.exists() {
            return Ok(None);
        }
        
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(_) => return Ok(None),
        };
        
        let mut contents = String::new();
        match file.read_to_string(&mut contents) {
            Ok(_) => {
                let trimmed = contents.trim();
                if let Some(captures) = SYMREF_REGEX.captures(trimmed) {
                    // It's a symbolic reference
                    if let Some(target_path) = captures.get(1) {
                        return Ok(Some(Reference::Symbolic(target_path.as_str().to_string())));
                    }
                }
                
                // It's a direct reference (OID)
                Ok(Some(Reference::Direct(trimmed.to_string())))
            },
            Err(_) => Ok(None),
        }
    }
    
    // Follow symbolic references to get the final OID
    pub fn read_symref(&self, path: &Path) -> Result<Option<String>, Error> {
        let ref_result = self.read_oid_or_symref(path)?;
        
        match ref_result {
            Some(Reference::Symbolic(target)) => {
                // Follow the symbolic reference
                self.read_symref(&self.pathname.join(target))
            },
            Some(Reference::Direct(oid)) => {
                // Return the OID directly
                Ok(Some(oid))
            },
            None => Ok(None),
        }
    }
    
    // Update a reference file with proper locking
    fn update_ref_file(&self, path: &Path, content: &str) -> Result<(), Error> {
        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                Error::DirectoryCreation(format!(
                    "Failed to create directory '{}': {}",
                    parent.display(),
                    e
                ))
            })?;
        }
        
        // Create a lockfile for safe writing
        let mut lockfile = Lockfile::new(path);
        
        // Acquire the lock
        let acquired = lockfile.hold_for_update()
            .map_err(|e| Error::Generic(format!("Lock error: {:?}", e)))?;
        
        if !acquired {
            return Err(Error::Generic(format!(
                "Could not acquire lock on '{}'", path.display()
            )));
        }
        
        // Write the content with a newline
        lockfile.write(&format!("{}\n", content))
            .map_err(|e| Error::Generic(format!("Write error: {:?}", e)))?;
        
        // Commit the changes
        lockfile.commit_ref()
            .map_err(|e| Error::Generic(format!("Commit error: {:?}", e)))?;
        
        Ok(())
    }
    
    // Update a symref, following it to its target
    fn update_symref(&self, path: &Path, oid: &str) -> Result<(), Error> {
        // Create a lockfile for safe writing
        let mut lockfile = Lockfile::new(path);
        
        // Acquire the lock
        let acquired = lockfile.hold_for_update()
            .map_err(|e| Error::Generic(format!("Lock error: {:?}", e)))?;
        
        if !acquired {
            return Err(Error::Generic(format!(
                "Could not acquire lock on '{}'", path.display()
            )));
        }
        
        // Read the current reference
        let ref_result = self.read_oid_or_symref(path)?;
        
        match ref_result {
            Some(Reference::Symbolic(target)) => {
                // Release this lock and follow the symref
                lockfile.rollback()?;
                self.update_symref(&self.pathname.join(target), oid)
            },
            Some(Reference::Direct(_)) | None => {
                // Write directly to this file
                lockfile.write(&format!("{}\n", oid))
                    .map_err(|e| Error::Generic(format!("Write error: {:?}", e)))?;
                
                lockfile.commit_ref()
                    .map_err(|e| Error::Generic(format!("Commit error: {:?}", e)))
            }
        }
    }
    
    // Check if a branch name is valid (not matching the invalid patterns)
    fn is_valid_branch_name(&self, name: &str) -> bool {
        // Define invalid patterns for branch names
        lazy_static::lazy_static! {
            static ref INVALID_NAME: Regex = Regex::new(r"(?x)
                ^\.|              # starts with .
                /\.|              # contains a path component starting with .
                \.\.|             # contains ..
                ^/|               # starts with /
                /$|               # ends with /
                /|                # contains slash anywhere
                \.lock$|          # ends with .lock
                @\{|              # contains @{
                [\x00-\x20*:?\[\\\^~\x7f] # contains control chars or special chars
            ").unwrap();
        }
        
        // Empty names are invalid
        if name.is_empty() {
            return false;
        }
        
        // Check against the invalid patterns
        !INVALID_NAME.is_match(name)
    }
    
    // Get current reference (HEAD or the branch it points to)
    pub fn current_ref(&self) -> Result<Reference, Error> {
        let head_path = self.pathname.join(HEAD);
        let ref_result = self.read_oid_or_symref(&head_path)?;
        
        match ref_result {
            Some(Reference::Symbolic(target)) => {
                // It's pointing to a branch
                Ok(Reference::Symbolic(target))
            },
            Some(Reference::Direct(oid)) => {
                // Detached HEAD
                Ok(Reference::Direct(oid))
            },
            None => {
                // No HEAD yet
                Ok(Reference::Direct(String::new()))
            }
        }
    }
    
    // Get short name for a reference path
    pub fn short_name(&self, path: &str) -> String {
        let path_buf = PathBuf::from(path);
        
        if path_buf.starts_with("refs/heads/") {
            // Remove refs/heads/ prefix for branch names
            path_buf.strip_prefix("refs/heads/")
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string())
        } else {
            path.to_string()
        }
    }
    
    // List all branches in the repository
    pub fn list_branches(&self) -> Result<Vec<Reference>, Error> {
        self.list_refs(&self.heads_path)
    }
    
    // List all refs in a directory, recursively
    fn list_refs(&self, dir: &Path) -> Result<Vec<Reference>, Error> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        
        let mut refs = Vec::new();
        
        match fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        
                        if path.is_dir() {
                            // Recursively list refs in subdirectories
                            let mut subrefs = self.list_refs(&path)?;
                            refs.append(&mut subrefs);
                        } else {
                            // Add this file as a reference
                            if let Some(relative) = path.strip_prefix(&self.pathname).ok() {
                                refs.push(Reference::Symbolic(relative.to_string_lossy().to_string()));
                            }
                        }
                    }
                }
            },
            Err(_) => {}
        }
        
        Ok(refs)
    }
    
    // Delete a branch and return its OID
    pub fn delete_branch(&self, branch_name: &str) -> Result<String, Error> {
        let branch_path = self.heads_path.join(branch_name);
        
        // Create a lockfile for safe deletion
        let mut lockfile = Lockfile::new(&branch_path);
        
        // Acquire the lock
        let acquired = lockfile.hold_for_update()
            .map_err(|e| Error::Generic(format!("Lock error: {:?}", e)))?;
        
        if !acquired {
            return Err(Error::Generic(format!(
                "Could not acquire lock on '{}'", branch_path.display()
            )));
        }
        
        // Read the OID before deleting
        let oid = match self.read_symref(&branch_path)? {
            Some(oid) => oid,
            None => {
                return Err(Error::Generic(format!(
                    "Branch '{}' not found.", branch_name
                )));
            }
        };
        
        // Delete the branch file
        fs::remove_file(&branch_path)
            .map_err(|e| Error::IO(e))?;
            
        // Clean up empty parent directories
        self.delete_parent_directories(&branch_path)?;
        
        // Release the lock
        lockfile.rollback()?;
        
        Ok(oid)
    }
    
    // Delete empty parent directories after removing a branch
    fn delete_parent_directories(&self, path: &Path) -> Result<(), Error> {
        let mut current = path.parent().map(|p| p.to_path_buf());
        
        while let Some(dir) = current {
            // Stop if we've reached the .git/refs/heads directory
            if dir == self.heads_path {
                break;
            }
            
            // Try to remove the directory
            match fs::remove_dir(&dir) {
                Ok(_) => {
                    // Successfully removed directory, continue to parent
                    current = dir.parent().map(|p| p.to_path_buf());
                },
                Err(e) => {
                    // If directory is not empty, stop
                    if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                        break;
                    }
                    // For other errors, report them
                    return Err(Error::IO(e));
                }
            }
        }
        
        Ok(())
    }
    
    // List all refs with a specific prefix
    pub fn list_refs_with_prefix(&self, prefix: &str) -> Result<Vec<Reference>, Error> {
        let mut refs = Vec::new();
        
        // Determinăm directorul bazat pe prefix
        let prefix_path = self.pathname.join(prefix);
        let prefix_dir = if prefix_path.is_dir() {
            prefix_path
        } else {
            // Dacă prefixul nu este un director, încercăm să obținem directorul părinte
            if let Some(parent) = prefix_path.parent() {
                parent.to_path_buf()
            } else {
                // Dacă nu există un părinte, folosim rădăcina refs
                self.refs_path.clone()
            }
        };
        
        // Verificăm dacă directorul există
        if !prefix_dir.exists() {
            return Ok(refs);
        }
        
        // Citim referințele din director, recursiv
        self.read_refs_with_prefix(&prefix_dir, prefix, &mut refs)?;
        
        Ok(refs)
    }
    
    // Internally read refs with a specific prefix
    fn read_refs_with_prefix(&self, dir: &Path, prefix: &str, refs: &mut Vec<Reference>) -> Result<(), Error> {
        if !dir.exists() {
            return Ok(());
        }
        
        match fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        let path_str = path.to_string_lossy().to_string();
                        
                        // Verificăm dacă începe cu prefixul dorit
                        if !path_str.contains(prefix) {
                            continue;
                        }
                        
                        if path.is_dir() {
                            // Recursiv pentru directoare
                            self.read_refs_with_prefix(&path, prefix, refs)?;
                        } else {
                            // Adăugăm referința dacă este un fișier
                            if let Some(relative) = path.strip_prefix(&self.pathname).ok() {
                                let relative_str = relative.to_string_lossy().to_string();
                                if relative_str.starts_with(prefix) {
                                    refs.push(Reference::Symbolic(relative_str));
                                }
                            }
                        }
                    }
                }
            },
            Err(e) => {
                return Err(Error::IO(e));
            }
        }
        
        Ok(())
    }
}