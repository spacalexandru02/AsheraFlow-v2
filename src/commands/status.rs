// src/commands/status.rs - With tree structure traversal debugging
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::core::color::Color;
use crate::core::database::database::Database;
use crate::core::database::blob::Blob;
use crate::core::database::entry::DatabaseEntry;
use crate::core::database::tree::{Tree, TreeEntry};
use crate::core::database::commit::Commit;
use crate::core::file_mode::FileMode;
use crate::core::index::entry::Entry;

use crate::core::index::index::Index;
use crate::core::refs::Refs;
use crate::core::workspace::Workspace;
use crate::errors::error::Error;
use crate::core::database::tree::TREE_MODE;

// Enum for change types
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
enum ChangeType {
    WorkspaceModified,
    WorkspaceDeleted,
    IndexAdded,
    IndexModified,
    IndexDeleted,
}

pub struct StatusCommand;

impl StatusCommand {
    /// Check if file metadata matches the index entry
    fn stat_match(entry: &Entry, stat: &fs::Metadata) -> bool {
        // Check file size
        let size_matches = entry.get_size() as u64 == stat.len();
        
        // Check file mode
        let entry_mode = entry.get_mode();
        let file_mode = Self::mode_for_stat(stat);
        let mode_matches = FileMode::are_equivalent(entry_mode.0, file_mode.0);
        
        size_matches && mode_matches
    }
    
    /// Check if file timestamps match the index entry
    fn times_match(entry: &Entry, stat: &fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            
            // Convert to seconds and nanoseconds for comparison
            let stat_mtime_sec = stat.mtime() as u32;
            let stat_mtime_nsec = stat.mtime_nsec() as u32;

            println!("Comparare timestamps pentru {}", entry.path);
            println!("Index mtime: {}.{}", entry.get_mtime(), entry.get_mtime_nsec());
            println!("File mtime: {}.{}", stat_mtime_sec, stat_mtime_nsec);
            
            // Compare modification times
            entry.get_mtime() == stat_mtime_sec && entry.get_mtime_nsec() == stat_mtime_nsec
        }
        
        #[cfg(not(unix))]
        {
            // On Windows, we don't have the same granularity, so convert to seconds
            if let Ok(mtime) = stat.modified() {
                if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                    let stat_mtime_sec = duration.as_secs() as u32;
                    return entry.get_mtime() == stat_mtime_sec;
                }
            }
            
            // If we can't get the modification time, assume they don't match
            false
        }
    }
    
    /// Determine file mode from metadata (executable vs regular)
    fn mode_for_stat(stat: &fs::Metadata) -> FileMode {
        FileMode::from_metadata(stat)
    }
    
    /// Check if a directory contains trackable files (recursively)
    fn is_trackable_dir(dir_path: &Path) -> Result<bool, Error> {
        if !dir_path.is_dir() {
            return Ok(false);
        }
        
        // Check if directory contains non-hidden files
        match std::fs::read_dir(dir_path) {
            Ok(entries) => {
                for entry_result in entries {
                    match entry_result {
                        Ok(entry) => {
                            let path = entry.path();
                            let file_name = entry.file_name();
                            
                            // Skip hidden files and directories
                            if let Some(name) = file_name.to_str() {
                                if name.starts_with('.') {
                                    continue;
                                }
                            }
                            
                            if path.is_file() {
                                // Found a trackable file
                                return Ok(true);
                            } else if path.is_dir() {
                                // Recursively check subdirectories
                                if Self::is_trackable_dir(&path)? {
                                    return Ok(true);
                                }
                            }
                        },
                        Err(e) => return Err(Error::IO(e)),
                    }
                }
                
                // No trackable files found
                Ok(false)
            },
            Err(e) => Err(Error::IO(e)),
        }
    }
    
    /// Get status for a specific path based on change types
    fn status_for(path: &str, changes: &HashMap<String, HashSet<ChangeType>>) -> String {
        let mut left = " ";
        let mut right = " ";
        
        if let Some(change_set) = changes.get(path) {
            // Status for first column (HEAD -> Index)
            if change_set.contains(&ChangeType::IndexAdded) {
                left = "A";
            } else if change_set.contains(&ChangeType::IndexModified) {
                left = "M";
            } else if change_set.contains(&ChangeType::IndexDeleted) {
                left = "D";
            }
            
            // Status for second column (Index -> Workspace)
            if change_set.contains(&ChangeType::WorkspaceDeleted) {
                right = "D";
            } else if change_set.contains(&ChangeType::WorkspaceModified) {
                right = "M";
            }
        }
        
        format!("{}{}", left, right)
    }
    
    
    /// Record a change for a specific path
    fn record_change(
        changed: &mut HashSet<String>,
        changes: &mut HashMap<String, HashSet<ChangeType>>,
        path: String,
        change_type: ChangeType
    ) {
        changed.insert(path.clone());
        changes.entry(path)
              .or_insert_with(HashSet::new)
              .insert(change_type);
    }

    /// Diagnostic function to inspect objects in the database
    fn diagnose_object(database: &mut Database, oid: &str) -> Result<(), Error> {
        println!("Diagnostic for object: {}", oid);
        
        // Try to load the object
        match database.load(oid) {
            Ok(obj) => {
                println!("  Successfully loaded object");
                println!("  Object type: {}", obj.get_type());
                
                // Try to cast to different types
                if let Some(tree) = obj.as_any().downcast_ref::<Tree>() {
                    println!("  Object is a Tree with {} entries", tree.get_entries().len());
                    
                    // Print the entries
                    for (name, entry) in tree.get_entries() {
                        match entry {
                            TreeEntry::Blob(entry_oid, mode) => {
                                println!("    Entry: {} (blob, mode {}) -> {}", name, mode, entry_oid);
                            },
                            TreeEntry::Tree(subtree) => {
                                if let Some(subtree_oid) = subtree.get_oid() {
                                    println!("    Entry: {} (tree) -> {}", name, subtree_oid);
                                } else {
                                    println!("    Entry: {} (tree) -> <no OID>", name);
                                }
                            }
                        }
                    }
                } else if let Some(_blob) = obj.as_any().downcast_ref::<Blob>() {
                    println!("  Object is a Blob");
                    
                    // Try to read and parse the blob as a tree
                    println!("  Attempting to parse blob as tree...");
                    let bytes = obj.to_bytes();
                    match Tree::parse(&bytes) {
                        Ok(tree) => {
                            println!("  Successfully parsed blob as tree with {} entries", tree.get_entries().len());
                            
                            // Print the entries
                            for (name, entry) in tree.get_entries() {
                                match entry {
                                    TreeEntry::Blob(entry_oid, mode) => {
                                        println!("    Entry: {} (blob, mode {}) -> {}", name, mode, entry_oid);
                                    },
                                    TreeEntry::Tree(subtree) => {
                                        if let Some(subtree_oid) = subtree.get_oid() {
                                            println!("    Entry: {} (tree) -> {}", name, subtree_oid);
                                        } else {
                                            println!("    Entry: {} (tree) -> <no OID>", name);
                                        }
                                    }
                                }
                            }
                        },
                        Err(e) => {
                            println!("  Failed to parse blob as tree: {}", e);
                        }
                    }
                } else if let Some(commit) = obj.as_any().downcast_ref::<Commit>() {
                    println!("  Object is a Commit");
                    println!("  Tree: {}", commit.get_tree());
                } else {
                    println!("  Object is of unknown type");
                }
            },
            Err(e) => {
                println!("  Failed to load object: {}", e);
            }
        }
        
        Ok(())
    }

    /// Load the HEAD tree with diagnostics
    fn load_head_tree(
        refs: &Refs,
        database: &mut Database
    ) -> Result<HashMap<String, DatabaseEntry>, Error> {
        let mut head_tree = HashMap::new();
        
        println!("Loading HEAD tree");
        
        // Read HEAD reference
        if let Some(head_oid) = refs.read_head()? {
            println!("HEAD OID: {}", head_oid);
            
            // Load the commit
            let commit_obj = match database.load(&head_oid) {
                Ok(obj) => {
                    println!("DEBUG: Successfully loaded commit object");
                    obj
                },
                Err(e) => {
                    println!("DEBUG: Failed to load commit: {}", e);
                    return Err(e);
                }
            };
            
            let commit = match commit_obj.as_any().downcast_ref::<Commit>() {
                Some(c) => {
                    println!("DEBUG: Successfully cast to Commit");
                    c
                },
                None => {
                    println!("DEBUG: Object is not a Commit");
                    return Err(Error::Generic("Object is not a commit".to_string()));
                }
            };
            
            let root_tree_oid = commit.get_tree();
            println!("Commit tree OID: {}", root_tree_oid);
            
            // Diagnose the root tree
            Self::diagnose_object(database, root_tree_oid)?;
            
            // Also diagnose the src directory if it exists
            if let Ok(root_obj) = database.load(root_tree_oid) {
                if let Some(root_tree) = root_obj.as_any().downcast_ref::<Tree>() {
                    for (name, entry) in root_tree.get_entries() {
                        if name == "src" {
                            match entry {
                                TreeEntry::Blob(oid, _) => {
                                    println!("Diagnosing src directory (blob):");
                                    Self::diagnose_object(database, oid)?;
                                },
                                TreeEntry::Tree(subtree) => {
                                    if let Some(oid) = subtree.get_oid() {
                                        println!("Diagnosing src directory (tree):");
                                        Self::diagnose_object(database, oid)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // Use a proper generic recursive traversal to build the complete head_tree
            Self::traverse_tree_structure(database, root_tree_oid, PathBuf::new(), &mut head_tree)?;
            
            println!("Found {} entries in HEAD tree", head_tree.len());
            for (path, entry) in &head_tree {
                println!("  {} -> {}", path, entry.get_oid());
            }
        } else {
            println!("No HEAD found, tree is empty");
        }
        
        Ok(head_tree)
    }
    /// Recursively traverse the tree structure
    /// Recursively traverse the tree structure with special handling for directories
    fn traverse_tree_structure(
        database: &mut Database,
        tree_oid: &str,
        prefix: PathBuf,
        head_tree: &mut HashMap<String, DatabaseEntry>
    ) -> Result<(), Error> {
        println!("Traversing tree: {} at path: {}", tree_oid, prefix.display());
        
        // Load the tree object
        let obj = database.load(tree_oid)?;
        
        if let Some(tree) = obj.as_any().downcast_ref::<Tree>() {
            // Process all entries in the tree
            for (name, entry) in tree.get_entries() {
                // Construct the full path for this entry
                let entry_path = if prefix.as_os_str().is_empty() {
                    PathBuf::from(name)
                } else {
                    prefix.join(name)
                };
                
                let path_str = entry_path.to_string_lossy().to_string();
                
                match entry {
                    TreeEntry::Blob(oid, mode) => {
                        // Store file entry in the head_tree
                        println!("  Found file in HEAD: {} -> {}", path_str, oid);
                        head_tree.insert(
                            path_str.clone(),
                            DatabaseEntry::new(
                                path_str,
                                oid.clone(),
                                &mode.to_octal_string()
                            )
                        );
                    },
                    TreeEntry::Tree(subtree) => {
                        if let Some(subtree_oid) = subtree.get_oid() {
                            println!("  Found directory in HEAD: {} -> {}", path_str, subtree_oid);
                            
                            // Store directory entry in the head_tree
                            head_tree.insert(
                                path_str.clone(),
                                DatabaseEntry::new(
                                    path_str.clone(),
                                    subtree_oid.clone(),
                                    &TREE_MODE.to_octal_string()
                                )
                            );
                            
                            // Recursively process the subtree
                            Self::traverse_tree_structure(database, subtree_oid, entry_path, head_tree)?;
                        }
                    }
                }
            }
        } else if obj.get_type() == "blob" {
            // Sometimes blobs are used to store directories (special handling)
            let blob_data = obj.to_bytes();
            if let Ok(parsed_tree) = Tree::parse(&blob_data) {
                println!("  Successfully parsed blob as tree with {} entries", parsed_tree.get_entries().len());
                
                // Process entries in the parsed tree
                for (name, entry) in parsed_tree.get_entries() {
                    let entry_path = if prefix.as_os_str().is_empty() {
                        PathBuf::from(name)
                    } else {
                        prefix.join(name)
                    };
                    
                    let path_str = entry_path.to_string_lossy().to_string();
                    
                    match entry {
                        TreeEntry::Blob(blob_oid, mode) => {
                            println!("  Found file in parsed tree: {} -> {}", path_str, blob_oid);
                            head_tree.insert(
                                path_str.clone(),
                                DatabaseEntry::new(
                                    path_str,
                                    blob_oid.clone(),
                                    &mode.to_octal_string()
                                )
                            );
                        },
                        TreeEntry::Tree(subtree) => {
                            if let Some(subtree_oid) = subtree.get_oid() {
                                println!("  Found directory in parsed tree: {} -> {}", path_str, subtree_oid);
                                head_tree.insert(
                                    path_str.clone(),
                                    DatabaseEntry::new(
                                        path_str.clone(),
                                        subtree_oid.clone(),
                                        &TREE_MODE.to_octal_string()
                                    )
                                );
                                
                                // Recursively process the subtree
                                Self::traverse_tree_structure(database, subtree_oid, entry_path, head_tree)?;
                            }
                        }
                    }
                }
            }
        }
        
        Ok(())
    }

    /// Improved method to check index entries against the HEAD tree
    fn check_index_against_head_tree(
        index_entry: &Entry,
        head_tree: &HashMap<String, DatabaseEntry>,
        changed: &mut HashSet<String>,
        changes: &mut HashMap<String, HashSet<ChangeType>>
    ) {
        let path = index_entry.get_path();
        
        println!("Comparing index with HEAD for {}", path);
        println!("  Index OID: {}", index_entry.get_oid());
        
        // If HEAD tree is empty (first commit case)
        if head_tree.is_empty() {
            println!("  HEAD tree is empty, marking file as added: {}", path);
            Self::record_change(changed, changes, path.to_string(), ChangeType::IndexAdded);
            return;
        }
        
        // Check if this file exists in HEAD
        if let Some(head_entry) = head_tree.get(path) {
            println!("  HEAD OID: {}", head_entry.get_oid());
            
            // Skip if this is a directory entry
            if Self::is_directory_from_mode(head_entry.get_mode()) {
                println!("  Skipping directory entry: {}", path);
                return;
            }
            
            // Compare OIDs
            let oids_match = index_entry.get_oid() == head_entry.get_oid();
            println!("  OIDs match: {}", oids_match);
            
            // Content comparison - if OIDs differ, file has been modified
            if !oids_match {
                println!("  Content changed (different OIDs), marking as modified");
                Self::record_change(changed, changes, path.to_string(), ChangeType::IndexModified);
            } else {
                println!("  File is unchanged in index");
            }
        } else {
            println!("  File not found in HEAD, marking as added: {}", path);
            Self::record_change(changed, changes, path.to_string(), ChangeType::IndexAdded);
        }
    }

    /// Improved method to check HEAD tree entries against the index
    fn check_head_tree_against_index(
        head_tree: &HashMap<String, DatabaseEntry>,
        index: &Index,
        changed: &mut HashSet<String>,
        changes: &mut HashMap<String, HashSet<ChangeType>>
    ) {
        // Skip this check if HEAD is empty
        if head_tree.is_empty() {
            println!("HEAD tree is empty, skipping deleted files check");
            return;
        }
        
        println!("Checking for files in HEAD that are missing from index");
        
        // Find entries that are in HEAD but not in index
        for (path, head_entry) in head_tree {
            // Skip if this is a directory
            if Self::is_directory_from_mode(head_entry.get_mode()) {
                println!("  Skipping directory entry: {}", path);
                continue;
            }
            
            // Check if this file exists in the index
            if !index.tracked(path) {
                // Check if this file is part of a directory that might be tracked in a different way
                if Self::is_parent_of_tracked_files(path, index) {
                    println!("  Directory {} contains tracked files, not marking as deleted", path);
                    continue;
                }
                
                println!("  File in HEAD but not in index: {}", path);
                Self::record_change(changed, changes, path.clone(), ChangeType::IndexDeleted);
            }
        }
    }

    // Helper method to determine if a mode string represents a directory
    fn is_directory_from_mode(mode_str: &str) -> bool {
        let mode = FileMode::parse(mode_str);
        mode.is_directory()
    }
    
    /// Check if a path is a parent of tracked files
    fn is_parent_of_tracked_files(path: &str, index: &Index) -> bool {
        // Ensure path ends with a slash for proper prefix matching
        let normalized_path = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{}/", path)
        };
        
        // Check if any file in the index has this path as a prefix
        index.entries.keys().any(|file_path| file_path.starts_with(&normalized_path))
    }
    
    /// Main execution method
    pub fn execute(porcelain: bool) -> Result<(), Error> {
        let start_time = Instant::now();
        
        // Initialize paths and components
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Check if .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository (or any of the parent directories): .ash directory not found".into()));
        }
        
        let workspace = Workspace::new(root_path);
        let mut database = Database::new(git_path.join("objects"));
        let mut index = Index::new(git_path.join("index"));
        let refs = Refs::new(&git_path);
        
        // Load the index (with lock for potential timestamp updates)
        if !index.load_for_update()? {
            return Err(Error::Generic("Failed to acquire lock on index file".to_string()));
        }
        
        // Load the HEAD tree with diagnostics
        let head_tree = Self::load_head_tree(&refs, &mut database)?;
        
        // Get tracked files from index
        let index_entries: HashMap<String, String> = index
            .each_entry()
            .map(|entry| (entry.get_path().to_string(), entry.get_oid().to_string()))
            .collect();
        
        // Prepare data structures for tracking changes
        let mut untracked = HashSet::new();  // Files in workspace but not in index
        let mut changed = HashSet::new();    // Files with any type of change
        let mut changes = HashMap::new();    // Map of path -> set of change types
        let mut stats_cache = HashMap::new(); // Cache for file metadata
        
        // Collect parent directories of tracked files
        let mut tracked_dirs = HashSet::new();
        for path in index_entries.keys() {
            let path_buf = PathBuf::from(path);
            let mut current = path_buf.clone();
            
            while let Some(parent) = current.parent() {
                if parent.as_os_str().is_empty() {
                    break;
                }
                tracked_dirs.insert(parent.to_path_buf());
                current = parent.to_path_buf();
            }
        }
        
        // Step 1: Scan workspace to find untracked files
        Self::scan_workspace(
            &workspace,
            &mut untracked,
            &index_entries,
            &tracked_dirs,
            root_path,
            &PathBuf::new(),
            &mut stats_cache
        )?;
        
        // Step 2: Compare index entries with HEAD
        for entry in index.each_entry() {
            Self::check_index_against_head_tree(
                entry,
                &head_tree,
                &mut changed,
                &mut changes
            );
        }
        
        // Step 3: Find files deleted from index (in HEAD but not in index)
        Self::check_head_tree_against_index(
            &head_tree,
            &index,
            &mut changed,
            &mut changes
        );
        
        // Step 4: Compare index entries with workspace (working tree changes)
        for (path, oid) in &index_entries {
            let path_buf = PathBuf::from(path);
            
            // Check if file exists
            if !workspace.path_exists(&path_buf)? {
                // File is in index but not in workspace (deleted)
                Self::record_change(&mut changed, &mut changes, path.clone(), ChangeType::WorkspaceDeleted);
                continue;
            }
            
            // Skip if already marked as untracked (shouldn't happen)
            if untracked.contains(path) {
                continue;
            }
            
            // Check if file is modified using cached metadata
            if let Some(metadata) = stats_cache.get(path) {
                // Get index entry for comparison
                let index_entry = index.get_entry(path).unwrap();
                
                // First quick check: compare file metadata (size and mode)
                if !Self::stat_match(index_entry, &metadata) {
                    Self::record_change(&mut changed, &mut changes, path.clone(), ChangeType::WorkspaceModified);
                    continue;
                }
                
                // Optimization: Check timestamps - if they match, assume content hasn't changed
                if Self::times_match(index_entry, &metadata) {
                    // Timestamps match, assume file hasn't changed
                    continue;
                }
                
                // If timestamps don't match, need to check content hash
                match workspace.read_file(&path_buf) {
                    Ok(data) => {
                        // Calculate hash using database
                        let computed_oid = database.hash_file_data(&data);
                        
                        println!("Verifying file: {}", path);
                        println!("  Index hash: {}", oid);
                        println!("  Computed hash: {}", computed_oid);
                        
                        if &computed_oid != oid {
                            // File has changed, mark as modified
                            Self::record_change(&mut changed, &mut changes, path.clone(), ChangeType::WorkspaceModified);
                        } else {
                            // File hasn't actually changed, just timestamps
                            // Update index entry with new timestamps to avoid re-reading next time
                            index.update_entry_stat(path, &metadata)?;
                        }
                    },
                    Err(_) => {
                        // If we can't read the file for any reason, consider it modified
                        Self::record_change(&mut changed, &mut changes, path.clone(), ChangeType::WorkspaceModified);
                    }
                }
            } else {
                // No metadata in cache for an indexed file, assume it's been deleted
                Self::record_change(&mut changed, &mut changes, path.clone(), ChangeType::WorkspaceDeleted);
            }
        }
        
        // Write any timestamp updates to index
        if index.is_changed() {
            index.write_updates()?;
        } else {
            // No changes to index, release lock
            index.rollback()?;
        }
        
        // Display results
        if porcelain {
            // Machine-readable output (--porcelain option)
            Self::print_porcelain(&untracked, &changed, &changes);
        } else {
            // Human-readable output
            Self::print_human_readable(&untracked, &changed, &changes);
        }
        
        let elapsed = start_time.elapsed();
        if !porcelain {
            println!("\n{} {:.2}s", Color::cyan("Status completed in"), elapsed.as_secs_f32());
        }
        
        Ok(())
    }

    fn scan_workspace(
        workspace: &Workspace,
        untracked: &mut HashSet<String>,
        index_entries: &HashMap<String, String>,
        tracked_dirs: &HashSet<PathBuf>,
        root_path: &Path,
        prefix: &Path,
        stats_cache: &mut HashMap<String, fs::Metadata>,
    ) -> Result<(), Error> {
        let current_path = if prefix.as_os_str().is_empty() {
            root_path.to_path_buf()
        } else {
            root_path.join(prefix)
        };
        
        // List files in current directory
        match std::fs::read_dir(&current_path) {
            Ok(entries) => {
                for entry_result in entries {
                    match entry_result {
                        Ok(entry) => {
                            let file_name = entry.file_name();
                            let entry_path = entry.path();
                            
                            // Skip .ash directory
                            if file_name == ".ash" {
                                continue;
                            }
                            
                            // Get relative path from root
                            let rel_path = if prefix.as_os_str().is_empty() {
                                PathBuf::from(file_name)
                            } else {
                                prefix.join(file_name)
                            };
                            
                            let rel_path_str = rel_path.to_string_lossy().to_string();
                            
                            // Check if path is tracked in index
                            let is_tracked = index_entries.contains_key(&rel_path_str);
                            let is_in_tracked_dir = tracked_dirs.contains(&rel_path);
                            
                            if entry_path.is_dir() {
                                if is_tracked || is_in_tracked_dir {
                                    // If directory is tracked or contains tracked files, 
                                    // scan it recursively
                                    Self::scan_workspace(
                                        workspace, 
                                        untracked, 
                                        index_entries, 
                                        tracked_dirs,
                                        root_path,
                                        &rel_path,
                                        stats_cache
                                    )?;
                                } else if Self::is_trackable_dir(&entry_path)? {
                                    // If directory contains trackable files, mark it
                                    untracked.insert(format!("{}/", rel_path_str));
                                }
                                // If directory is empty or contains only ignored files, skip it
                            } else if !is_tracked {
                                // File is not tracked in index
                                untracked.insert(rel_path_str);
                            } else {
                                // File is tracked - cache metadata for later comparisons
                                if let Ok(metadata) = entry_path.metadata() {
                                    stats_cache.insert(rel_path_str, metadata);
                                }
                            }
                        },
                        Err(e) => return Err(Error::IO(e)),
                    }
                }
            },
            Err(e) => return Err(Error::IO(e)),
        }
        
        Ok(())
    }
    
    fn print_porcelain(
        untracked: &HashSet<String>,
        changed: &HashSet<String>,
        changes: &HashMap<String, HashSet<ChangeType>>,
    ) {
        // Collect all files to sort them
        let mut all_files: Vec<String> = Vec::new();
        
        // Add changed files
        for path in changed {
            all_files.push(path.clone());
        }
        
        // Add untracked files
        for path in untracked {
            all_files.push(path.clone());
        }
        
        // Sort all files
        all_files.sort();
        
        // Display status for each file
        for path in &all_files {
            if untracked.contains(path) {
                println!("{} {}", Color::red("??"), Color::red(path));
            } else {
                let status = Self::status_for(path, changes);
                let status_colored = if status.contains('M') {
                    Color::yellow(&status)
                } else if status.contains('A') {
                    Color::green(&status)
                } else if status.contains('D') {
                    Color::red(&status)
                } else {
                    status.to_string()
                };
                println!("{} {}", status_colored, path);
            }
        }
    }
    
    fn print_human_readable(
        untracked: &HashSet<String>,
        changed: &HashSet<String>,
        changes: &HashMap<String, HashSet<ChangeType>>,
    ) {
        // Group changes by type
        let mut changes_to_be_committed = Vec::new();
        let mut changes_not_staged = Vec::new();
        
        for path in changed {
            if let Some(change_set) = changes.get(path) {
                // Changes between HEAD and index
                if change_set.contains(&ChangeType::IndexAdded) {
                    changes_to_be_committed.push((path, "new file"));
                } else if change_set.contains(&ChangeType::IndexModified) {
                    changes_to_be_committed.push((path, "modified"));
                } else if change_set.contains(&ChangeType::IndexDeleted) {
                    changes_to_be_committed.push((path, "deleted"));
                }
                
                // Changes between index and workspace
                if change_set.contains(&ChangeType::WorkspaceModified) {
                    changes_not_staged.push((path, "modified"));
                } else if change_set.contains(&ChangeType::WorkspaceDeleted) {
                    changes_not_staged.push((path, "deleted"));
                }
            }
        }
        
        println!("On branch {}", Color::green("master"));
        
        // Display changes in index (HEAD -> Index)
        if !changes_to_be_committed.is_empty() {
            println!("\n{}:", Color::green("Changes to be committed"));
            println!("  (use \"{}\" to unstage)", Color::cyan("ash reset HEAD <file>..."));
            
            // Sort for consistent output
            changes_to_be_committed.sort();
            
            for (path, status) in &changes_to_be_committed {
                let colored_status = match *status {
                    "new file" => Color::green("new file"),
                    "modified" => Color::green("modified"),
                    "deleted" => Color::green("deleted"),
                    _ => status.to_string()
                };
                println!("        {}: {}", colored_status, Color::green(path));
            }
        }
        
        // Display changes in workspace (Index -> Workspace)
        if !changes_not_staged.is_empty() {
            println!("\n{}:", Color::red("Changes not staged for commit"));
            println!("  (use \"{}\" to update what will be committed)", Color::cyan("ash add <file>..."));
            println!("  (use \"{}\" to discard changes in working directory)", Color::cyan("ash checkout -- <file>..."));
            
            // Sort for consistent output
            changes_not_staged.sort();
            
            for (path, status) in &changes_not_staged {
                let colored_status = match *status {
                    "modified" => Color::red("modified"),
                    "deleted" => Color::red("deleted"),
                    _ => status.to_string()
                };
                println!("        {}: {}", colored_status, Color::red(path));
            }
        }
        
        // Display untracked files
        if !untracked.is_empty() {
            println!("\n{}:", Color::red("Untracked files"));
            println!("  (use \"{}\" to include in what will be committed)", Color::cyan("ash add <file>..."));
            
            let mut sorted_untracked: Vec<&String> = untracked.iter().collect();
            sorted_untracked.sort();
            
            for path in sorted_untracked {
                println!("        {}", Color::red(path));
            }
        }
        
        // If no changes, show "working tree clean" message
        if changes_to_be_committed.is_empty() && changes_not_staged.is_empty() && untracked.is_empty() {
            println!("{}", Color::green("nothing to commit, working tree clean"));
        }
    }
}