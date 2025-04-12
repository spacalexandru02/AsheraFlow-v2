// src/commands/add.rs - With improved directory handling
use std::path::{Path, PathBuf};
use std::collections::{HashSet, HashMap};
use std::time::Instant;
use crate::core::database::blob::Blob;
use crate::core::database::database::Database;
use crate::core::database::tree::{Tree, TreeEntry, TREE_MODE};
use crate::core::database::commit::Commit;
use crate::core::index::index::Index;
use crate::core::workspace::Workspace;
use crate::core::refs::Refs;
use crate::errors::error::Error;
use std::fs;

pub struct AddCommand;

impl AddCommand {
    pub fn execute(paths: &[String]) -> Result<(), Error> {
        let start_time = Instant::now();
        
        if paths.is_empty() {
            return Err(Error::Generic("No paths specified for add command".into()));
        }
    
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository (or any of the parent directories): .ash directory not found".into()));
        }
        
        let workspace = Workspace::new(root_path);
        let mut database = Database::new(git_path.join("objects"));
        let mut index = Index::new(git_path.join("index"));
        let refs = Refs::new(&git_path);
        
        // Prepare a set to deduplicate files (in case of overlapping path arguments)
        let mut files_to_add: HashSet<PathBuf> = HashSet::new();
        let mut files_to_delete: HashSet<String> = HashSet::new();
        let mut had_missing_valid_files = false;
        
        // Try to acquire the lock on the index
        if !index.load_for_update()? {
            return Err(Error::Lock(format!(
                "Unable to acquire lock on index. Another process may be using it. \
                If not, the .ash/index.lock file may need to be manually removed."
            )));
        }
        
        // Get current files in index to avoid unnecessary operations
        let mut existing_oids = HashMap::new();
        for entry in index.each_entry() {
            existing_oids.insert(entry.get_path().to_string(), entry.oid.clone());
        }
        
        // Flag to track if we have deleted directories
        let mut has_deleted_dirs = false;
        
        // Check each path
        for path_str in paths {
            let path = PathBuf::from(path_str);
            
            // Check if the path exists in the workspace
            if !workspace.path_exists(&path)? {
                // Path doesn't exist in workspace, check if it's in the index
                let rel_path_str = if path.is_absolute() {
                    match path.strip_prefix(root_path) {
                        Ok(rel) => rel.to_string_lossy().to_string(),
                        Err(_) => path.to_string_lossy().to_string()
                    }
                } else {
                    path.to_string_lossy().to_string()
                };
                
                // Handle case where the path is exactly a file in the index
                if existing_oids.contains_key(&rel_path_str) {
                    println!("File {} has been deleted, will remove from index", rel_path_str);
                    files_to_delete.insert(rel_path_str);
                    continue;
                }
                
                // Check if the path is a directory prefix for any files in the index
                let prefix_to_check = if rel_path_str.ends_with('/') {
                    rel_path_str.clone()
                } else {
                    format!("{}/", rel_path_str)
                };
                
                let mut has_matches = false;
                
                // Find all entries that start with this prefix (meaning they're in this directory)
                for key in existing_oids.keys() {
                    if key.starts_with(&prefix_to_check) || key == &rel_path_str {
                        println!("Found index entry {} under directory {}, will remove", key, rel_path_str);
                        files_to_delete.insert(key.clone());
                        has_matches = true;
                    }
                }
                
                if has_matches {
                    has_deleted_dirs = true;
                    continue;
                }
                
                // If we get here, the path wasn't found in the workspace or index
                println!("fatal: pathspec '{}' did not match any files", path_str);
                had_missing_valid_files = true;
                continue;
            }
            
            // Path exists, proceed with normal processing
            match workspace.list_files_from(&path, &existing_oids) {
                Ok((found_files, missing_files)) => {
                    if found_files.is_empty() && missing_files.is_empty() {
                        println!("warning: '{}' didn't match any files", path_str);
                    } else {
                        // Add found files to set
                        for file in found_files {
                            files_to_add.insert(file);
                        }
                        
                        // Add missing files to set for deletion
                        for file in missing_files {
                            files_to_delete.insert(file);
                        }
                    }
                },
                Err(Error::InvalidPath(_)) => {
                    println!("fatal: pathspec '{}' did not match any files", path_str);
                    had_missing_valid_files = true;
                },
                Err(e) => return Err(e),
            }
        }
        
        // If any paths were invalid (not in workspace or index), exit without modifying the index
        if had_missing_valid_files && !has_deleted_dirs && files_to_add.is_empty() && files_to_delete.is_empty() {
            index.rollback()?;
            return Err(Error::Generic("Adding files failed: some paths don't exist".into()));
        }
        
        // If no files were found to add or delete, exit early
        if files_to_add.is_empty() && files_to_delete.is_empty() {
            index.rollback()?;
            println!("No files to add or remove");
            return Ok(());
        }
        
        // Track the number of files we successfully process
        let mut added_count = 0;
        let mut deleted_count = 0;
        let mut unchanged_count = 0;
        
        // First, handle deleted files
        for path_str in &files_to_delete {
            if index.entries.remove(path_str).is_some() {
                index.keys.remove(path_str);
                index.changed = true;
                deleted_count += 1;
                println!("Removed {} from index", path_str);
            }
        }
        
        // Create a buffer for batch processing
        let mut blobs_to_save: Vec<(PathBuf, Vec<u8>, fs::Metadata)> = Vec::with_capacity(files_to_add.len());
        
        // First pass: read all files and check for errors before we start modifying anything
        for file_path in &files_to_add {
            // Try to read file content and metadata
            match (
                workspace.read_file(file_path),
                workspace.stat_file(file_path)
            ) {
                (Ok(data), Ok(stat)) => {
                    // Check if file is already in index with same content
                    let file_key = file_path.to_string_lossy().to_string();
                    
                    // Pre-compute hash to check if the file has changed
                    let new_oid = database.hash_file_data(&data);
                    
                    if let Some(old_oid) = existing_oids.get(&file_key) {
                        if old_oid == &new_oid {
                            // File exists in index with same content, skip it
                            unchanged_count += 1;
                            continue;
                        }
                    }
                    
                    // Queue file for processing
                    blobs_to_save.push((file_path.clone(), data, stat));
                },
                (Err(Error::IO(e)), _) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    // Permission denied error
                    index.rollback()?;
                    return Err(Error::Generic(format!(
                        "error: open('{}'): Permission denied\nfatal: adding files failed",
                        file_path.display()
                    )));
                },
                (Err(e), _) => {
                    // Other read errors
                    index.rollback()?;
                    return Err(Error::Generic(format!(
                        "error: Failed to read '{}': {}\nfatal: adding files failed",
                        file_path.display(), e
                    )));
                },
                (_, Err(e)) => {
                    // Metadata errors
                    index.rollback()?;
                    return Err(Error::Generic(format!(
                        "error: Failed to get stats for '{}': {}\nfatal: adding files failed",
                        file_path.display(), e
                    )));
                }
            }
        }
        
        // Second pass: process all files that need to be updated
        for (file_path, data, stat) in blobs_to_save {
            // Create and store the blob
            let mut blob = Blob::new(data);
            if let Err(e) = database.store(&mut blob) {
                // Release the lock if we fail to store the blob
                index.rollback()?;
                return Err(Error::Generic(format!(
                    "Failed to store blob for '{}': {}", file_path.display(), e
                )));
            }
            
            // Get the OID
            let oid = match blob.get_oid() {
                Some(id) => id,
                None => {
                    // Release the lock if the blob has no OID
                    index.rollback()?;
                    return Err(Error::Generic(
                        "Blob OID not set after storage".into()
                    ));
                }
            };
            
            // Add to index
            if let Err(e) = index.add(&file_path, oid, &stat) {
                index.rollback()?;
                return Err(e);
            }
            
            added_count += 1;
        }
        
        // Write index updates
        if added_count > 0 || deleted_count > 0 {
            if index.write_updates()? {
                let elapsed = start_time.elapsed();
                
                // Get all files from HEAD commit with proper tree traversal
                let mut head_files = HashMap::<String, String>::new(); // path -> oid
                
                // Only load from HEAD if we have a commit
                if let Ok(Some(head_oid)) = refs.read_head() {
                    println!("Examining HEAD commit: {}", head_oid);
                    
                    if let Ok(commit_obj) = database.load(&head_oid) {
                        if let Some(commit) = commit_obj.as_any().downcast_ref::<Commit>() {
                            let root_tree_oid = commit.get_tree();
                            println!("Root tree OID: {}", root_tree_oid);
                            
                            // Recursively collect all files from HEAD tree
                            Self::collect_files_from_tree(&mut database, root_tree_oid, PathBuf::new(), &mut head_files)?;
                            
                            println!("Found {} files in HEAD", head_files.len());
                        }
                    }
                }
                
                // Count how many files are new vs modified
                let mut new_files = 0;
                let mut modified_files = 0;
                
                for path in &files_to_add {
                    let path_str = path.to_string_lossy().to_string();
                    
                    if head_files.contains_key(&path_str) {
                        // Get current OID from index
                        let current_oid = index.get_entry(&path_str)
                            .map(|entry| entry.get_oid())
                            .unwrap_or("");
                        
                        // Compare OIDs to see if the file has changed
                        if let Some(head_oid) = head_files.get(&path_str) {
                            if head_oid != current_oid {
                                modified_files += 1;
                            }
                        }
                    } else {
                        new_files += 1;
                    }
                }
                
                // Format output message
                let mut message = String::new();
                
                if new_files > 0 {
                    message.push_str(&format!(
                        "{} new file{}", 
                        new_files,
                        if new_files == 1 { "" } else { "s" }
                    ));
                }
                
                if modified_files > 0 {
                    if !message.is_empty() {
                        message.push_str(" and ");
                    }
                    message.push_str(&format!(
                        "{} modified file{}", 
                        modified_files,
                        if modified_files == 1 { "" } else { "s" }
                    ));
                }
                
                if deleted_count > 0 {
                    if !message.is_empty() {
                        message.push_str(" and ");
                    }
                    message.push_str(&format!(
                        "{} deleted file{}", 
                        deleted_count,
                        if deleted_count == 1 { "" } else { "s" }
                    ));
                }
                
                if message.is_empty() {
                    message = format!("{} file{}", added_count + deleted_count, 
                        if (added_count + deleted_count) == 1 { "" } else { "s" });
                }
                
                if unchanged_count > 0 {
                    println!(
                        "{} added to index, {} file{} unchanged ({:.2}s)",
                        message,
                        unchanged_count,
                        if unchanged_count == 1 { "" } else { "s" },
                        elapsed.as_secs_f32()
                    );
                } else {
                    println!(
                        "{} added to index ({:.2}s)",
                        message,
                        elapsed.as_secs_f32()
                    );
                }
                
                Ok(())
            } else {
                Err(Error::Generic("Failed to update index".into()))
            }
        } else if unchanged_count > 0 {
            // If we didn't add any files, release the lock
            index.rollback()?;
            println!(
                "No files changed, {} file{} already up to date",
                unchanged_count,
                if unchanged_count == 1 { "" } else { "s" }
            );
            Ok(())
        } else {
            // If we didn't add any files, release the lock
            index.rollback()?;
            println!("No changes were made to the index");
            Ok(())
        }
    }

    // Recursively collect all files from a tree and its subtrees
    fn collect_files_from_tree(
        database: &mut Database,
        tree_oid: &str,
        prefix: PathBuf,
        files: &mut HashMap<String, String>
    ) -> Result<(), Error> {
        println!("Traversing tree: {} at path: {}", tree_oid, prefix.display());
        
        // Load the object
        let obj = database.load(tree_oid)?;
        
        // Check if the object is a tree
        if let Some(tree) = obj.as_any().downcast_ref::<Tree>() {
            // Process each entry in the tree
            for (name, entry) in tree.get_entries() {
                let entry_path = if prefix.as_os_str().is_empty() {
                    PathBuf::from(name)
                } else {
                    prefix.join(name)
                };
                
                let entry_path_str = entry_path.to_string_lossy().to_string();
                
                match entry {
                    TreeEntry::Blob(oid, mode) => {
                        // If this is a directory entry masquerading as a blob
                        if *mode == TREE_MODE || mode.is_directory() {
                            println!("Found directory stored as blob: {} -> {}", entry_path_str, oid);
                            // Recursively process this directory
                            Self::collect_files_from_tree(database, oid, entry_path, files)?;
                        } else {
                            // Regular file
                            println!("Found file: {} -> {}", entry_path_str, oid);
                            files.insert(entry_path_str, oid.clone());
                        }
                    },
                    TreeEntry::Tree(subtree) => {
                        if let Some(subtree_oid) = subtree.get_oid() {
                            println!("Found directory: {} -> {}", entry_path_str, subtree_oid);
                            // Recursively process this directory
                            Self::collect_files_from_tree(database, subtree_oid, entry_path, files)?;
                        } else {
                            println!("Warning: Tree entry without OID: {}", entry_path_str);
                        }
                    }
                }
            }
            
            return Ok(());
        }
        
        // If object is a blob, try to parse it as a tree
        if obj.get_type() == "blob" {
            println!("Object is a blob, attempting to parse as tree...");
            
            // Attempt to parse blob as a tree (this handles directories stored as blobs)
            let blob_data = obj.to_bytes();
            match Tree::parse(&blob_data) {
                Ok(parsed_tree) => {
                    println!("Successfully parsed blob as tree with {} entries", parsed_tree.get_entries().len());
                    
                    // Process each entry in the parsed tree
                    for (name, entry) in parsed_tree.get_entries() {
                        let entry_path = if prefix.as_os_str().is_empty() {
                            PathBuf::from(name)
                        } else {
                            prefix.join(name)
                        };
                        
                        let entry_path_str = entry_path.to_string_lossy().to_string();
                        
                        match entry {
                            TreeEntry::Blob(oid, mode) => {
                                if *mode == TREE_MODE || mode.is_directory() {
                                    println!("Found directory in parsed tree: {} -> {}", entry_path_str, oid);
                                    // Recursively process this directory
                                    Self::collect_files_from_tree(database, oid, entry_path, files)?;
                                } else {
                                    println!("Found file in parsed tree: {} -> {}", entry_path_str, oid);
                                    files.insert(entry_path_str, oid.clone());
                                }
                            },
                            TreeEntry::Tree(subtree) => {
                                if let Some(subtree_oid) = subtree.get_oid() {
                                    println!("Found directory in parsed tree: {} -> {}", entry_path_str, subtree_oid);
                                    // Recursively process this directory
                                    Self::collect_files_from_tree(database, subtree_oid, entry_path, files)?;
                                } else {
                                    println!("Warning: Tree entry without OID in parsed tree: {}", entry_path_str);
                                }
                            }
                        }
                    }
                    
                    return Ok(());
                },
                Err(e) => {
                    // If we're at a non-root path, this might be a file
                    if !prefix.as_os_str().is_empty() {
                        let path_str = prefix.to_string_lossy().to_string();
                        println!("Adding file at path: {} -> {}", path_str, tree_oid);
                        files.insert(path_str, tree_oid.to_string());
                        return Ok(());
                    }
                    
                    println!("Failed to parse blob as tree: {}", e);
                }
            }
        }
        
        // Special case for top-level entries that might need deeper traversal
        // This handles cases where we have entries like "src" but need to explore "src/commands"
        if prefix.as_os_str().is_empty() {
            // Check all found entries in the root
            for (path, oid) in files.clone() {  // Clone to avoid borrowing issues
                // Only look at top-level directory entries (no path separators)
                if !path.contains('/') {
                    println!("Checking top-level entry for deeper traversal: {} -> {}", path, oid);
                    
                    // Try to load and traverse it as a directory
                    let dir_path = PathBuf::from(&path);
                    if let Err(e) = Self::collect_files_from_tree(database, &oid, dir_path, files) {
                        println!("Error traversing {}: {}", path, e);
                        // Continue with other entries even if this one fails
                    }
                }
            }
        }
        
        println!("Object {} is neither a tree nor a blob that can be parsed as a tree", tree_oid);
        Ok(())
    }
}