// src/core/repository/inspector.rs
use std::path::Path;
use std::collections::HashMap;
use crate::errors::error::Error;
use crate::core::database::blob::Blob;
use crate::core::database::entry::DatabaseEntry;
use crate::core::index::entry::Entry;
use crate::core::workspace::Workspace;
use crate::core::index::index::Index;
use crate::core::database::database::{Database, GitObject};

// Enum for change types in the repository
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ChangeType {
    Untracked,
    Modified,
    Added,
    Deleted,
}

// The Inspector takes references to components it needs
pub struct Inspector<'a> {
    workspace: &'a Workspace,
    index: &'a Index,
    database: &'a Database,
}

impl<'a> Inspector<'a> {
    // Constructor takes separate components
    pub fn new(workspace: &'a Workspace, index: &'a Index, database: &'a Database) -> Self {
        Inspector { 
            workspace,
            index,
            database,
        }
    }
    
    /// Check if a path is an untracked file or directory containing untracked files
    pub fn trackable_file(&self, path: &Path, stat: &std::fs::Metadata) -> Result<bool, Error> {
        // If it's a file, check if it's in the index
        if stat.is_file() {
            // Get the path as string
            let path_str = path.to_string_lossy().to_string();
            
            // If the file is not in the index, it's trackable
            return Ok(!self.index.tracked(&path_str));
        }
        
        // If it's a directory, check if it contains any untracked files
        if stat.is_dir() {
            // Get all files in the directory
            return self.directory_contains_untracked(path);
        }
        
        // Not a file or directory (e.g., symlink), consider not trackable
        Ok(false)
    }
    
    /// Check if a directory contains any untracked files
    fn directory_contains_untracked(&self, dir_path: &Path) -> Result<bool, Error> {
        if !dir_path.is_dir() {
            return Ok(false);
        }
        
        println!("DEBUG: Checking if directory contains untracked files: {}", dir_path.display());
        
        // Get all entries in the directory
        match std::fs::read_dir(dir_path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            let path = entry.path();
                            let file_name = entry.file_name();
                            
                            // Skip hidden files and .ash directory
                            let name_str = file_name.to_string_lossy();
                            if name_str.starts_with('.') || name_str == ".ash" {
                                continue;
                            }
                            
                            // Get relative path from workspace root
                            let rel_path = match path.strip_prefix(&self.workspace.root_path) {
                                Ok(p) => p,
                                Err(_) => continue,
                            };
                            
                            let rel_path_str = rel_path.to_string_lossy().to_string();
                            
                            // If it's a file not in the index, it's untracked
                            if path.is_file() && !self.index.tracked(&rel_path_str) {
                                println!("DEBUG: Found untracked file: {}", rel_path_str);
                                return Ok(true);
                            } else if path.is_dir() {
                                // Recursively check subdirectories
                                if self.directory_contains_untracked(&path)? {
                                    return Ok(true);
                                }
                            }
                        },
                        Err(_) => continue,
                    }
                }
                Ok(false)
            },
            Err(e) => Err(Error::IO(e)),
        }
    }
    
    /// Compare an index entry to a file in the workspace
    pub fn compare_index_to_workspace(&self, 
                                     entry: Option<&Entry>, 
                                     stat: Option<&std::fs::Metadata>) -> Result<Option<ChangeType>, Error> {
        // File not in index but exists in workspace
        if entry.is_none() {
            return Ok(Some(ChangeType::Untracked));
        }
        
        let entry = entry.unwrap();
        
        // File in index but not in workspace
        if stat.is_none() {
            return Ok(Some(ChangeType::Deleted));
        }
        
        let stat = stat.unwrap();
        
        // Skip metadata checks and go directly to content comparison
        // Read file content
        let path = Path::new(&entry.path);
        let data = match self.workspace.read_file(path) {
            Ok(d) => d,
            Err(e) => {
                println!("WARNING: Failed to read file for comparison: {} - {}", path.display(), e);
                return Ok(Some(ChangeType::Modified)); // Assume modified if we can't read it
            }
        };
        
        // Calculate hash of content
        let actual_oid = self.database.hash_file_data(&data);
        
        // Compare with expected OID
        let content_changed = entry.oid != actual_oid;
        
        // Debug output to help diagnose issues
        if content_changed {
            println!("DEBUG: File {} content differs", path.display());
            println!("  Index OID:   {}", entry.oid);
            println!("  Content OID: {}", actual_oid);
            return Ok(Some(ChangeType::Modified));
        }
        
        Ok(None) // No change
    }
    
    /// Compare a tree entry to an index entry
    pub fn compare_tree_to_index(&self, item: Option<&DatabaseEntry>, entry: Option<&Entry>) -> Option<ChangeType> {
        // Neither exists
        if item.is_none() && entry.is_none() {
            return None;
        }
        
        // Item in tree but not in index
        if item.is_some() && entry.is_none() {
            return Some(ChangeType::Deleted);
        }
        
        // Item not in tree but in index
        if item.is_none() && entry.is_some() {
            return Some(ChangeType::Added);
        }
        
        // Both exist, compare mode and OID
        let item = item.unwrap();
        let entry = entry.unwrap();
        
        // Compare mode and object ID
        let mode_match = item.get_mode() == entry.mode_octal();
        let oid_match = item.get_oid() == entry.oid;
        
        if !mode_match || !oid_match {
            println!("DEBUG: Entry differs - mode match: {}, oid match: {}", mode_match, oid_match);
            println!("  Tree mode: {}, Index mode: {}", item.get_mode(), entry.mode_octal());
            println!("  Tree OID:  {}, Index OID:  {}", item.get_oid(), entry.oid);
            Some(ChangeType::Modified)
        } else {
            None
        }
    }

    /// Read a file from the workspace and compare with a stored blob
    pub fn compare_workspace_vs_blob(&self, path: &Path, oid: &str) -> Result<bool, Error> {
        // Read the file from workspace
        let workspace_data = match self.workspace.read_file(path) {
            Ok(data) => data,
            Err(_) => return Ok(true), // If we can't read, consider it different
        };
        
        // Calculate hash
        let workspace_oid = self.database.hash_file_data(&workspace_data);
        
        // Compare OIDs
        let matches = workspace_oid == oid;
        
        if !matches {
            println!("DEBUG: File {} differs from blob {}", path.display(), oid);
            println!("  Blob OID:      {}", oid);
            println!("  Workspace OID: {}", workspace_oid);
        }
        
        Ok(!matches) // Return true if they differ
    }
    
    /// Check if a file in workspace has uncommitted changes
    pub fn has_uncommitted_changes(&self, path: &Path) -> Result<bool, Error> {
        // Get the entry from the index
        let path_str = path.to_string_lossy().to_string();
        let entry = self.index.get_entry(&path_str);
        
        if entry.is_none() {
            // Not in index - consider untracked
            return Ok(true);
        }
        
        // Get file metadata
        let stat = match self.workspace.stat_file(path) {
            Ok(s) => s,
            Err(_) => return Ok(true), // If we can't stat, consider it changed
        };
        
        // Check if content differs
        let compare_result = self.compare_index_to_workspace(entry, Some(&stat))?;
        
        Ok(compare_result.is_some())
    }
    
    /// Analyze all changes in the workspace compared to index
    pub fn analyze_workspace_changes(&self) -> Result<HashMap<String, ChangeType>, Error> {
        let mut changes = HashMap::new();
        
        // Check all entries in the index
        for entry in self.index.each_entry() {
            let path = Path::new(entry.get_path());
            
            // Check if file exists in workspace
            let stat_result = self.workspace.stat_file(path);
            
            // Fixed type error: Here we need to correctly handle the Metadata reference
            match stat_result {
                Ok(stat) => {
                    // Compare with workspace - use direct stat reference
                    let result = self.compare_index_to_workspace(Some(entry), Some(&stat))?;
                    
                    if let Some(change_type) = result {
                        changes.insert(entry.get_path().to_string(), change_type);
                    }
                },
                Err(_) => {
                    // File doesn't exist - mark as deleted
                    changes.insert(entry.get_path().to_string(), ChangeType::Deleted);
                }
            }
        }
        
        // Now find untracked files
        // This would require scanning the workspace but isn't needed for checkout
        // We'll leave this as a future enhancement
        
        Ok(changes)
    }
}