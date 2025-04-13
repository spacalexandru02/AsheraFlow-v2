// src/core/repository/migration.rs
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use crate::core::database::blob::Blob;
use crate::core::database::tree::{Tree, TreeEntry};
use crate::core::file_mode::FileMode;
use crate::errors::error::Error;
use crate::core::repository::repository::Repository;
use crate::core::database::entry::DatabaseEntry;
use crate::core::repository::inspector::{Inspector, ChangeType};

// Define conflict types for different error scenarios
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum ConflictType {
    StaleFile,           // Local changes would be overwritten
    StaleDirectory,      // Directory contains modified files
    UntrackedOverwritten, // Untracked file would be overwritten
    UntrackedRemoved,    // Untracked file would be removed
    UncommittedChanges,  // Added a new type for uncommitted changes
}

pub struct Migration<'a> {
    pub repo: &'a mut Repository,
    pub diff: HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>,
    pub errors: Vec<String>,
    conflicts: HashMap<ConflictType, HashSet<String>>,
    changes_to_make: Vec<Change>,
}

#[derive(Clone)]
enum Change {
    Create { path: PathBuf, entry: DatabaseEntry },
    Update { path: PathBuf, entry: DatabaseEntry },
    Delete { path: PathBuf },
}

impl<'a> Migration<'a> {
    pub fn new(repo: &'a mut Repository, tree_diff: HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>) -> Self {
        // Initialize conflict types
        let mut conflicts = HashMap::new();
        conflicts.insert(ConflictType::StaleFile, HashSet::new());
        conflicts.insert(ConflictType::StaleDirectory, HashSet::new());
        conflicts.insert(ConflictType::UntrackedOverwritten, HashSet::new());
        conflicts.insert(ConflictType::UntrackedRemoved, HashSet::new());
        conflicts.insert(ConflictType::UncommittedChanges, HashSet::new()); // Add the new conflict type
        
        Migration {
            repo,
            diff: tree_diff,
            errors: Vec::new(),
            conflicts,
            changes_to_make: Vec::new(),
        }
    }
    

    pub fn apply_changes(&mut self) -> Result<(), Error> {
        // Analyze changes using Inspector to detect conflicts
        self.analyze_changes()?;
        
        // Check if there are any conflicts that would prevent checkout
        self.check_conflicts()?;
        
        // Apply the planned changes
        self.execute_changes()?;
        
        // Final phase: perform a comprehensive cleanup of empty directories
        self.cleanup_empty_directories()?;
        
        Ok(())
    }
    
    // New method to perform more comprehensive directory cleanup
    fn cleanup_empty_directories(&mut self) -> Result<(), Error> {
        println!("Performing final empty directory cleanup");
        
        // First get all directories that exist in the workspace
        let workspace_dirs = self.find_all_workspace_directories()?;
        
        // Sort directories by depth (deepest first) to ensure proper cleanup
        let mut sorted_dirs: Vec<_> = workspace_dirs.into_iter().collect();
        sorted_dirs.sort_by(|a, b| {
            let a_depth = a.components().count();
            let b_depth = b.components().count();
            b_depth.cmp(&a_depth) // Descending order - deepest first
        });
        
        // Try to remove each directory if it's empty
        for dir in sorted_dirs {
            // Skip the root directory
            if dir.as_os_str().is_empty() || dir.to_string_lossy() == "." {
                continue;
            }
            
            let full_path = self.repo.workspace.root_path.join(&dir);
            
            // Skip if directory doesn't exist
            if !full_path.exists() || !full_path.is_dir() {
                continue;
            }
            
            // Check if directory is empty or contains only hidden files
            let is_effectively_empty = if let Ok(entries) = std::fs::read_dir(&full_path) {
                !entries
                    .filter_map(Result::ok)
                    .any(|e| {
                        let name = e.file_name();
                        let name_str = name.to_string_lossy();
                        !name_str.starts_with('.')
                    })
            } else {
                false
            };
            
            if is_effectively_empty {
                println!("Removing empty directory in final cleanup: {}", dir.display());
                
                // First try normal directory removal
                match std::fs::remove_dir(&full_path) {
                    Ok(_) => {
                        println!("Successfully removed empty directory: {}", dir.display());
                    },
                    Err(e) => {
                        // If that fails, try force removal for directories that might have hidden files
                        println!("Standard removal failed, trying force removal: {} - {}", dir.display(), e);
                        
                        // First remove any hidden files
                        if let Ok(entries) = std::fs::read_dir(&full_path) {
                            for entry in entries.filter_map(Result::ok) {
                                let entry_path = entry.path();
                                let name = entry.file_name();
                                let name_str = name.to_string_lossy();
                                
                                if name_str.starts_with('.') && entry_path.is_file() {
                                    if let Err(e) = std::fs::remove_file(&entry_path) {
                                        println!("Warning: Failed to remove hidden file: {} - {}", entry_path.display(), e);
                                    }
                                }
                            }
                        }
                        
                        // Try removal again
                        if let Err(e) = std::fs::remove_dir(&full_path) {
                            println!("Warning: Still could not remove directory: {} - {}", dir.display(), e);
                        } else {
                            println!("Successfully removed directory after clearing hidden files: {}", dir.display());
                        }
                    }
                }
            }
        }
        
        Ok(())
    }

    fn find_all_workspace_directories(&self) -> Result<HashSet<PathBuf>, Error> {
        let mut dirs = HashSet::new();
        let root_path = &self.repo.workspace.root_path;
        
        // Skip .ash directory
        let git_dir = root_path.join(".ash");
        
        self.collect_directories_recursive(root_path, &PathBuf::new(), &mut dirs, &git_dir)?;
        
        Ok(dirs)
    }
    
    // Helper to recursively collect directories
    fn collect_directories_recursive(
        &self, 
        full_path: &Path, 
        rel_path: &Path, 
        dirs: &mut HashSet<PathBuf>,
        git_dir: &Path
    ) -> Result<(), Error> {
        // Skip if this is the .ash directory
        if full_path == git_dir {
            return Ok(());
        }
        
        // Skip if path doesn't exist or isn't a directory
        if !full_path.exists() || !full_path.is_dir() {
            return Ok(());
        }
        
        // Add this directory
        dirs.insert(rel_path.to_path_buf());
        
        // Process subdirectories
        if let Ok(entries) = std::fs::read_dir(full_path) {
            for entry_result in entries {
                if let Ok(entry) = entry_result {
                    let entry_path = entry.path();
                    let entry_name = entry.file_name();
                    
                    // Skip hidden directories
                    if entry_name.to_string_lossy().starts_with('.') {
                        continue;
                    }
                    
                    // Only process directories
                    if entry_path.is_dir() {
                        // Get relative path
                        let entry_rel_path = if rel_path.as_os_str().is_empty() {
                            PathBuf::from(entry_name)
                        } else {
                            rel_path.join(entry_name)
                        };
                        
                        // Recursively collect this directory
                        self.collect_directories_recursive(&entry_path, &entry_rel_path, dirs, git_dir)?;
                    }
                }
            }
        }
        
        Ok(())
    }
    
    // Helper method to find all potentially empty directories in the workspace
    fn find_all_empty_directories(&self) -> Result<HashSet<PathBuf>, Error> {
        let mut dirs = HashSet::new();
        
        // Get all files from index
        for entry in self.repo.index.each_entry() {
            let path = PathBuf::from(entry.get_path());
            
            // Add all parent directories
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent.as_os_str().is_empty() || parent.to_string_lossy() == "." {
                    break;
                }
                dirs.insert(parent.to_path_buf());
                current = parent.parent();
            }
        }
        
        Ok(dirs)
    }
    
    fn analyze_changes(&mut self) -> Result<(), Error> {
        println!("Analyzing changes for migration");
        
        // Create Inspector to help analyze the repository state
        let inspector = Inspector::new(
            &self.repo.workspace,
            &self.repo.index,
            &self.repo.database
        );
        
        // First, check if there are uncommitted changes in the workspace
        // This is the key improvement - using the analyze_workspace_changes method
        let workspace_changes = inspector.analyze_workspace_changes()?;
        
        // If there are any uncommitted changes, record them as conflicts
        if !workspace_changes.is_empty() {
            println!("Found uncommitted changes in workspace:");
            for (path, change_type) in &workspace_changes {
                match change_type {
                    ChangeType::Modified | ChangeType::Added | ChangeType::Deleted => {
                        println!("  {} - {:?}", path, change_type);
                        self.conflicts.get_mut(&ConflictType::UncommittedChanges).unwrap().insert(path.clone());
                    },
                    _ => {} // Ignore untracked files here
                }
            }
            
            // If we found uncommitted changes, we can exit early
            if !self.conflicts.get(&ConflictType::UncommittedChanges).unwrap().is_empty() {
                return Ok(());
            }
        }
        
        // Next, find all files in current state that should be deleted
        let mut current_paths = HashSet::new();
        let mut target_paths = HashSet::new();
        
        // Clone diff to avoid borrowing issues
        let diff_clone = self.diff.clone();
        
        // Populate current and target path sets
        for (path, (old_entry, new_entry)) in &diff_clone {
            if old_entry.is_some() {
                current_paths.insert(path.clone());
            }
            if new_entry.is_some() {
                target_paths.insert(path.clone());
            }
        }
        
        // Find files that should be deleted (in current but not in target)
        let deleted_files: Vec<_> = current_paths.difference(&target_paths).cloned().collect();
        
        // Add deletions to our change list
        for path in deleted_files {
            println!("Planning deletion for file: {}", path.display());
            self.changes_to_make.push(Change::Delete { path });
        }
        
        // Now process all other changes
        for (path, (old_entry, new_entry)) in diff_clone {
            // Skip files that we're already planning to delete
            if new_entry.is_none() && old_entry.is_some() {
                // This is a deletion, already handled above
                continue;
            }
            
            // Skip directories from conflict check
            let is_directory = new_entry.as_ref().map_or(false, |e| {
                e.get_mode() == "040000" || FileMode::parse(e.get_mode()).is_directory()
            });
            
            if !is_directory {
                // Check for conflicts using Inspector
                let path_str = path.to_string_lossy().to_string();
                let entry = self.repo.index.get_entry(&path_str);
                
                // Check if index differs from both old and new versions
                if let Some(index_entry) = entry {
                    // Using Inspector to check tree-to-index relationships
                    let changed_from_old = inspector.compare_tree_to_index(old_entry.as_ref(), Some(index_entry));
                    let changed_from_new = inspector.compare_tree_to_index(new_entry.as_ref(), Some(index_entry));
                    
                    if changed_from_old.is_some() && changed_from_new.is_some() {
                        // Index has changes compared to both old and new - conflict
                        println!("Index entry for {} differs from both old and new trees", path_str);
                        self.conflicts.get_mut(&ConflictType::StaleFile).unwrap().insert(path_str.clone());
                        continue;
                    }
                    
                    // Use compare_workspace_vs_blob to check if workspace content matches the indexed content
                    if let Ok(has_changes) = inspector.compare_workspace_vs_blob(&path, index_entry.get_oid()) {
                        if has_changes {
                            println!("Uncommitted changes in workspace file: {}", path_str);
                            self.conflicts.get_mut(&ConflictType::StaleFile).unwrap().insert(path_str.clone());
                            continue;
                        }
                    }
                } else if self.repo.workspace.path_exists(&path)? {
                    // Untracked file in workspace - check for conflict
                    let stat = self.repo.workspace.stat_file(&path)?;
                    
                    if stat.is_file() {
                        if new_entry.is_some() {
                            // Would overwrite untracked file
                            println!("Untracked file would be overwritten: {}", path_str);
                            self.conflicts.get_mut(&ConflictType::UntrackedOverwritten).unwrap().insert(path_str.clone());
                            continue;
                        }
                    } else if stat.is_dir() {
                        // Check for untracked files in directory using Inspector
                        if inspector.trackable_file(&path, &stat)? {
                            println!("Directory contains untracked files: {}", path_str);
                            self.conflicts.get_mut(&ConflictType::StaleDirectory).unwrap().insert(path_str.clone());
                            continue;
                        }
                    }
                }
            }
            
            // No conflicts, plan the change
            if let Some(entry) = new_entry {
                if old_entry.is_some() {
                    // Update existing file
                    self.changes_to_make.push(Change::Update {
                        path: path.clone(),
                        entry,
                    });
                } else {
                    // Create new file
                    self.changes_to_make.push(Change::Create {
                        path: path.clone(),
                        entry,
                    });
                }
            }
        }
        
        Ok(())
    }
    
    // Check for conflicts and return appropriate error if any found
    fn check_conflicts(&mut self) -> Result<(), Error> {
        // Error messages for each conflict type
        let messages = HashMap::from([
            (ConflictType::StaleFile, (
                "Your local changes to the following files would be overwritten by checkout:",
                "Please commit your changes or stash them before you switch branches."
            )),
            (ConflictType::StaleDirectory, (
                "Updating the following directories would lose untracked files in them:",
                "\n"
            )),
            (ConflictType::UntrackedOverwritten, (
                "The following untracked working tree files would be overwritten by checkout:",
                "Please move or remove them before you switch branches."
            )),
            (ConflictType::UntrackedRemoved, (
                "The following untracked working tree files would be removed by checkout:",
                "Please move or remove them before you switch branches."
            )),
            (ConflictType::UncommittedChanges, (
                "You have uncommitted changes in your working tree:",
                "Please commit your changes or stash them before you switch branches."
            ))
        ]);
        
        // Check each conflict type
        for (conflict_type, paths) in &self.conflicts {
            if paths.is_empty() {
                continue;
            }
            
            // Get header and footer for this conflict type
            let (header, footer) = messages.get(conflict_type).unwrap();
            
            // Format the paths
            let mut sorted_paths: Vec<_> = paths.iter().collect();
            sorted_paths.sort();
            
            let mut lines = Vec::new();
            for path in sorted_paths {
                lines.push(format!("\t{}", path));
            }
            
            // Build the error message
            let mut error_message = String::new();
            error_message.push_str(header);
            error_message.push('\n');
            for line in lines {
                error_message.push_str(&line);
                error_message.push('\n');
            }
            error_message.push_str(footer);
            
            self.errors.push(error_message);
        }
        
        // If we have errors, we cannot proceed
        if !self.errors.is_empty() {
            return Err(Error::Generic("Checkout failed due to conflicts".to_string()));
        }
        
        Ok(())
    }
    
    // Execute all planned changes
    // In src/core/repository/migration.rs
// Modified execute_changes method for better directory cleanup

fn execute_changes(&mut self) -> Result<(), Error> {
    println!("Executing {} changes", self.changes_to_make.len());
    
    // Clone the changes to avoid borrowing issues
    let changes_clone = self.changes_to_make.clone();
    
    // Keep track of directories that might need cleanup
    let mut affected_dirs = HashSet::new();
    
    // First, handle deletions
    for change in &changes_clone {
        if let Change::Delete { path } = change {
            println!("Removing file: {}", path.display());
            self.repo.workspace.remove_file(path)?;
            
            // Also remove from index
            let path_str = path.to_string_lossy().to_string();
            self.repo.index.remove(&PathBuf::from(&path_str))?;
            
            // Add parent directories to the affected dirs list
            if let Some(parent) = path.parent() {
                if !(parent.as_os_str().is_empty() || parent.to_string_lossy() == ".") {
                    affected_dirs.insert(parent.to_path_buf());
                }
            }
        }
    }
    
    // Find all directories needed for new/updated files
    let mut needed_dirs = HashSet::new();
    for change in &changes_clone {
        match change {
            Change::Create { path, .. } | Change::Update { path, .. } => {
                // Add all parent directories
                let mut current = path.parent();
                while let Some(parent) = current {
                    if parent.as_os_str().is_empty() || parent.to_string_lossy() == "." {
                        break;
                    }
                    needed_dirs.insert(parent.to_path_buf());
                    current = parent.parent();
                }
            },
            _ => {}
        }
    }
    
    // Sort the directories by path length to ensure we create them in order
    let mut dir_list: Vec<_> = needed_dirs.iter().cloned().collect();
    dir_list.sort_by_key(|p| p.to_string_lossy().len());
    
    // Create all needed directories
    for dir in dir_list {
        println!("Creating directory: {}", dir.display());
        self.repo.workspace.make_directory(&dir)?;
    }
    
    // Now apply file creations and updates
    for change in changes_clone {
        match change {
            Change::Create { path, entry } | Change::Update { path, entry } => {
                // Check if this is a directory entry
                if entry.get_mode() == "040000" || FileMode::parse(entry.get_mode()).is_directory() {
                    println!("Creating directory: {}", path.display());
                    self.repo.workspace.make_directory(&path)?;
                    
                    // Process directory contents
                    self.process_directory_contents(&path, &entry.get_oid())?;
                } else {
                    // Write the file and update index
                    println!("Writing file: {}", path.display());
                    self.write_file(&path, &entry)?;
                }
            },
            _ => {}
        }
    }
    
    // Clean up affected directories - we'll use the improved recursive method 
    // which will automatically clean up parent directories as well
    for dir in affected_dirs {
        // Skip if this directory is needed for new/updated files
        if needed_dirs.contains(&dir) {
            continue;
        }
        
        println!("Checking if directory is empty: {}", dir.display());
        self.repo.workspace.remove_directory(&dir)?;
    }
    
    Ok(())
}
    
    // Write a file to the workspace and update the index
    fn write_file(&mut self, path: &Path, entry: &DatabaseEntry) -> Result<(), Error> {
        // Get blob contents
        let blob_obj = self.repo.database.load(&entry.get_oid())?;
        let blob_data = blob_obj.to_bytes();
        
        // Write to workspace
        self.repo.workspace.write_file(path, &blob_data)?;
        
        // Update index
        if let Ok(stat) = self.repo.workspace.stat_file(path) {
            self.repo.index.add(path, &entry.get_oid(), &stat)?;
        }
        
        Ok(())
    }
    
    // Process a directory's contents recursively
    // In src/core/repository/migration.rs
// Improved process_directory_contents method for better directory cleanup

    fn process_directory_contents(&mut self, directory_path: &Path, directory_oid: &str) -> Result<(), Error> {
        println!("Processing directory contents: {}", directory_path.display());
        
        // Load the tree object
        let obj = self.repo.database.load(directory_oid)?;
        
        // Make sure it's a tree
        if let Some(tree) = obj.as_any().downcast_ref::<Tree>() {
            // Build a comprehensive list of all files that should exist in target state
            let mut target_files = HashMap::new();
            
            // First, collect all files that should exist in this directory and subdirectories in the target state
            self.collect_all_target_files(tree, directory_path, &mut target_files)?;
            
            // Now, get current files in the workspace
            let current_files = self.get_all_workspace_files(directory_path)?;
            
            // Debug output
            println!("Target files for {}: {}", directory_path.display(), target_files.len());
            for (path, (oid, _)) in &target_files {
                println!("  Target file: {} -> {}", path.display(), oid);
            }
            
            println!("Current files for {}: {}", directory_path.display(), current_files.len());
            for path in &current_files {
                println!("  Current file: {}", path.display());
            }
            
            // First ensure all directories exist
            let mut directories = HashSet::new();
            for path in target_files.keys() {
                if let Some(parent) = path.parent() {
                    // Skip the top directory
                    if parent != directory_path {
                        directories.insert(parent.to_path_buf());
                    }
                }
            }
            
            // Sort directories by depth to create parent dirs first
            let mut dir_list: Vec<_> = directories.into_iter().collect();
            dir_list.sort_by_key(|p| p.components().count());
            
            // Create all necessary directories
            for dir in dir_list {
                println!("Creating directory: {}", dir.display());
                self.repo.workspace.make_directory(&dir)?;
            }
            
            // Now create/update all target files
            for (path, (oid, _)) in &target_files {
                // Create parent directories if needed
                if let Some(parent) = path.parent() {
                    if parent != directory_path && !parent.exists() {
                        println!("Creating parent directory: {}", parent.display());
                        self.repo.workspace.make_directory(parent)?;
                    }
                }
                
                // Write the file content
                println!("Writing file: {}", path.display());
                
                // Get and write the blob content
                let blob_obj = self.repo.database.load(oid)?;
                let blob_data = blob_obj.to_bytes();
                self.repo.workspace.write_file(path, &blob_data)?;
                
                // Update index
                if let Ok(stat) = self.repo.workspace.stat_file(path) {
                    self.repo.index.add(path, oid, &stat)?;
                }
            }
            
            // Find files that exist in current state but not in target state
            let files_to_remove: Vec<_> = current_files
                .difference(&target_files.keys().cloned().collect())
                .cloned()
                .collect();
            
            // Sort files by depth (deepest first) to avoid issues with removing parent dirs first
            let mut sorted_files_to_remove = files_to_remove.clone();
            sorted_files_to_remove.sort_by(|a, b| {
                let a_depth = a.components().count();
                let b_depth = b.components().count();
                b_depth.cmp(&a_depth) // Descending order - deepest first
            });
            
            // Delete files that exist in current state but not in target state
            for file_path in sorted_files_to_remove {
                println!("Removing file that doesn't exist in target: {}", file_path.display());
                self.repo.workspace.remove_file(&file_path)?;
                
                // Also remove from index
                let path_str = file_path.to_string_lossy().to_string();
                self.repo.index.remove(&PathBuf::from(&path_str))?;
            }
        }
        
        Ok(())
    }

    fn get_current_entries_in_dir(&self, dir_path: &Path) -> Result<HashSet<PathBuf>, Error> {
        let mut entries = HashSet::new();
        let full_dir_path = self.repo.workspace.root_path.join(dir_path);
        
        // Skip if directory doesn't exist
        if !full_dir_path.exists() || !full_dir_path.is_dir() {
            return Ok(entries);
        }
        
        // Add this directory's files and subdirectories recursively
        self.collect_entries_recursive(&full_dir_path, dir_path, &mut entries)?;
        
        Ok(entries)
    }

    fn collect_entries_recursive(&self, full_path: &Path, rel_path: &Path, entries: &mut HashSet<PathBuf>) -> Result<(), Error> {
        // Skip if path doesn't exist
        if !full_path.exists() {
            return Ok(());
        }
        
        // Add this entry if it's not the root directory we're checking
        if rel_path.as_os_str().len() > 0 {
            entries.insert(rel_path.to_path_buf());
        }
        
        // If it's a directory, process contents
        if full_path.is_dir() {
            if let Ok(dir_entries) = std::fs::read_dir(full_path) {
                for entry_result in dir_entries {
                    if let Ok(entry) = entry_result {
                        let entry_path = entry.path();
                        let entry_name = entry.file_name();
                        
                        // Skip hidden files and directories
                        if entry_name.to_string_lossy().starts_with('.') {
                            continue;
                        }
                        
                        // Get relative path
                        let entry_rel_path = rel_path.join(entry_name);
                        
                        // Recursively collect this entry
                        self.collect_entries_recursive(&entry_path, &entry_rel_path, entries)?;
                    }
                }
            }
        }
        
        Ok(())
    }

    // Get all current files in a specific directory
    fn get_current_files_in_dir(&self, dir_path: &Path) -> Result<HashSet<PathBuf>, Error> {
        let mut files = HashSet::new();
        let dir_prefix = dir_path.to_string_lossy().to_string();
        
        // Get files from index that match this directory
        for entry in self.repo.index.each_entry() {
            let path = PathBuf::from(entry.get_path());
            
            if (path.starts_with(dir_path) || 
                entry.get_path().starts_with(&dir_prefix)) &&
               path != *dir_path {
                files.insert(path);
            }
        }
        
        Ok(files)
    }

    fn get_all_workspace_files(&mut self, dir_path: &Path) -> Result<HashSet<PathBuf>, Error> {
        let mut files = HashSet::new();
        let full_dir_path = self.repo.workspace.root_path.join(dir_path);
        
        // Skip if directory doesn't exist
        if !full_dir_path.exists() || !full_dir_path.is_dir() {
            return Ok(files);
        }
        
        self.collect_files_recursive(&full_dir_path, dir_path, &mut files)?;
        
        Ok(files)
    }

    fn collect_files_recursive(&mut self, full_path: &Path, rel_path: &Path, files: &mut HashSet<PathBuf>) -> Result<(), Error> {
        // Skip if path doesn't exist
        if !full_path.exists() {
            return Ok(());
        }
        
        // If it's a file, add it
        if full_path.is_file() {
            files.insert(rel_path.to_path_buf());
            return Ok(());
        }
        
        // If it's a directory, process contents
        if full_path.is_dir() {
            if let Ok(dir_entries) = std::fs::read_dir(full_path) {
                for entry_result in dir_entries {
                    if let Ok(entry) = entry_result {
                        let entry_path = entry.path();
                        let entry_name = entry.file_name();
                        
                        // Skip .ash directory and hidden files
                        if entry_name.to_string_lossy() == ".ash" || 
                        entry_name.to_string_lossy().starts_with('.') {
                            continue;
                        }
                        
                        // Get relative path
                        let entry_rel_path = rel_path.join(entry_name);
                        
                        // Process this entry
                        self.collect_files_recursive(&entry_path, &entry_rel_path, files)?;
                    }
                }
            }
        }
        
        Ok(())
    }

    fn collect_all_target_files(
        &mut self,
        tree: &Tree,
        base_path: &Path,
        target_files: &mut HashMap<PathBuf, (String, FileMode)>
    ) -> Result<(), Error> {
        // Process each entry in the tree
        for (name, entry) in tree.get_entries() {
            let entry_path = base_path.join(name);
            
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    if mode.is_directory() {
                        // It's a directory stored as a blob, load it and process recursively
                        let subtree_obj = self.repo.database.load(oid)?;
                        if let Some(subtree) = subtree_obj.as_any().downcast_ref::<Tree>() {
                            self.collect_all_target_files(subtree, &entry_path, target_files)?;
                        } else if let Ok(subtree) = Tree::parse(&subtree_obj.to_bytes()) {
                            // Try parsing as a tree in case it's stored as a blob
                            self.collect_all_target_files(&subtree, &entry_path, target_files)?;
                        }
                    } else {
                        // It's a file, add to target files
                        target_files.insert(entry_path, (oid.clone(), *mode));
                    }
                },
                TreeEntry::Tree(subtree) => {
                    if let Some(subtree_oid) = subtree.get_oid() {
                        // It's a directory, load and process recursively
                        let subtree_obj = self.repo.database.load(subtree_oid)?;
                        if let Some(subtree) = subtree_obj.as_any().downcast_ref::<Tree>() {
                            self.collect_all_target_files(subtree, &entry_path, target_files)?;
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
}