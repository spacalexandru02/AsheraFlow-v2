use std::path::PathBuf;
use std::{env, path::Path, time::Instant};
use std::collections::{HashMap, HashSet};
use crate::core::database::tree::{TreeEntry, TREE_MODE};
use crate::{core::{database::{author::Author, commit::Commit, database::Database, entry::DatabaseEntry, tree::Tree}, index::index::Index, refs::Refs}, errors::error::Error};
pub struct CommitCommand;

impl CommitCommand {
    pub fn execute(message: &str) -> Result<(), Error> {
        let start_time = Instant::now();
        
        // Validate the commit message
        if message.trim().is_empty() {
            return Err(Error::Generic("Aborting commit due to empty commit message".into()));
        }
        
        println!("Starting commit execution");
        
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository (or any of the parent directories): .ash directory not found".into()));
        }
        
        let db_path = git_path.join("objects");
        
        println!("Initializing components");
        let mut database = Database::new(db_path);
        
        // Check for the index file
        let index_path = git_path.join("index");
        if !index_path.exists() {
            return Err(Error::Generic("No index file found. Please add some files first.".into()));
        }
        
        // Check for existing index.lock file before trying to load the index
        let index_lock_path = git_path.join("index.lock");
        if index_lock_path.exists() {
            return Err(Error::Lock(format!(
                "Unable to create '.ash/index.lock': File exists.\n\
                Another ash process seems to be running in this repository.\n\
                If it still fails, a process may have crashed in this repository earlier:\n\
                remove the file manually to continue."
            )));
        }
        
        let mut index = Index::new(index_path);
        
        println!("Loading index");
        // Load the index (read-only is sufficient for commit)
        match index.load() {
            Ok(_) => println!("Index loaded successfully"),
            Err(e) => return Err(Error::Generic(format!("Error loading index: {}", e))),
        }
        
        // Check if the index is empty
        if index.entries.is_empty() {
            return Err(Error::Generic("No changes staged for commit. Use 'ash add' to add files.".into()));
        }
        
        // Check for HEAD lock
        let head_lock_path = git_path.join("HEAD.lock");
        if head_lock_path.exists() {
            return Err(Error::Lock(format!(
                "Unable to create '.ash/HEAD.lock': File exists.\n\
                Another ash process seems to be running in this repository.\n\
                If it still fails, a process may have crashed in this repository earlier:\n\
                remove the file manually to continue."
            )));
        }
        
        let refs = Refs::new(&git_path);
        
        println!("Reading HEAD");
        // Get the parent commit OID
        let parent = match refs.read_head() {
            Ok(p) => {
                println!("HEAD read successfully: {:?}", p);
                p
            },
            Err(e) => {
                println!("Error reading HEAD: {:?}", e);
                return Err(e);
            }
        };
        
        // Convert index entries to database entries
        let database_entries: Vec<DatabaseEntry> = index.each_entry()
            .map(|index_entry| {
                DatabaseEntry::new(
                    index_entry.path.clone(),
                    index_entry.oid.clone(),
                    &index_entry.mode_octal()
                )
            })
            .collect();
        
        // Add this at the beginning of CommitCommand::execute to properly validate the paths
        println!("Index entries before building tree:");
        for entry in &database_entries {
            println!("  Path: {}  OID: {}  Mode: {}", entry.get_name(), entry.get_oid(), entry.get_mode());
        }
        
        // Verify all objects exist in the database
        let mut missing_objects = Vec::new();
        let mut unique_oids = HashSet::new();
        
        for entry in &database_entries {
            let oid = entry.get_oid();
            if !unique_oids.contains(oid) && !database.exists(oid) {
                missing_objects.push((oid.to_string(), entry.get_name().to_string()));
                unique_oids.insert(oid.to_string());
            }
        }
        
        if !missing_objects.is_empty() {
            let mut error_msg = String::from("Error: The following objects are missing from the object database:\n");
            for (oid, path) in missing_objects {
                error_msg.push_str(&format!("  {} {}\n", oid, path));
            }
            error_msg.push_str("\nAborting commit. Run 'ash add' on these files to add them to the object database.");
            return Err(Error::Generic(error_msg));
        }
        
        // Build tree from index entries
        let mut root = match Tree::build(database_entries.iter()) {
            Ok(tree) => tree,
            Err(e) => return Err(Error::Generic(format!("Failed to build tree: {}", e))),
        };
        
        // Add this right after the Tree::build call
        println!("\nTree structure after building:");
        println!("Root entries: {}", root.get_entries().len());
        for (name, entry) in root.get_entries() {
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    println!("  {} (blob, mode {}) -> {}", name, mode, oid);
                },
                TreeEntry::Tree(subtree) => {
                    let oid_str = if let Some(oid) = subtree.get_oid() {
                        format!("Some(\"{}\")", oid)
                    } else {
                        "None".to_string()
                    };
                    println!("  {} (tree) -> {}", name, oid_str);
                    
                    // Recursively print the first level of the subtree
                    for (sub_name, sub_entry) in subtree.get_entries() {
                        match sub_entry {
                            TreeEntry::Blob(sub_oid, sub_mode) => {
                                println!("    {}/{} (blob, mode {}) -> {}", name, sub_name, sub_mode, sub_oid);
                            },
                            TreeEntry::Tree(sub_subtree) => {
                                let sub_oid_str = if let Some(oid) = sub_subtree.get_oid() {
                                    format!("Some(\"{}\")", oid)
                                } else {
                                    "None".to_string()
                                };
                                println!("    {}/{} (tree) -> {}", name, sub_name, sub_oid_str);
                            }
                        }
                    }
                }
            }
        }
        
        // Store all trees
        println!("\nStoring trees to database...");
        let mut tree_counter = 0;
        if let Err(e) = root.traverse(|tree| {
            tree_counter += 1;
            println!("Storing tree #{} with {} entries...", tree_counter, tree.get_entries().len());
            
            // Debug: Print entries before storing
            for (name, entry) in tree.get_entries() {
                match entry {
                    TreeEntry::Blob(oid, mode) => {
                        println!("  Entry: {} (blob, mode {}) -> {}", name, mode, oid);
                    },
                    TreeEntry::Tree(subtree) => {
                        if let Some(oid) = subtree.get_oid() {
                            println!("  Entry: {} (tree) -> {}", name, oid);
                        } else {
                            println!("  Entry: {} (tree) -> <no OID>", name);
                        }
                    }
                }
            }
            
            match database.store(tree) {
                Ok(oid) => {
                    println!("  Tree stored with OID: {}", oid);
                    println!("  Verified: Tree now has OID: {}", tree.get_oid().unwrap_or(&"<none>".to_string()));
                    Ok(())
                },
                Err(e) => {
                    println!("  Error storing tree: {}", e);
                    Err(e)
                }
            }
        }) {
            return Err(Error::Generic(format!("Failed to store trees: {}", e)));
        }
        
        // Get the root tree OID
        let tree_oid = root.get_oid()
            .ok_or(Error::Generic("Tree OID not set after storage".into()))?;
        
        // With this fixed version:
        println!("\nChecking stored tree structure:");
        let stored_tree_obj = database.load(&tree_oid)?;
        let stored_tree = stored_tree_obj.as_any().downcast_ref::<Tree>().unwrap();
        println!("Stored root entries: {}", stored_tree.get_entries().len());
        for (name, entry) in stored_tree.get_entries() {
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    println!("  {} (blob, mode {}) -> {}", name, mode, oid);
                },
                TreeEntry::Tree(subtree) => {
                    let oid_str = if let Some(oid) = subtree.get_oid() {
                        format!("Some(\"{}\")", oid)
                    } else {
                        "None".to_string()
                    };
                    println!("  {} (tree) -> {}", name, oid_str);
                }
            }
        }
        
        // Adaugă aceste linii de debug
        println!("Tree OID: {}", tree_oid);
        if let Some(parent_oid) = &parent {
            println!("Parent OID: {}", parent_oid);
        }
        println!("Message: {}", message);
        
        // Create and store the commit
        let name = match env::var("GIT_AUTHOR_NAME").or_else(|_| env::var("USER")) {
            Ok(name) => name,
            Err(_) => return Err(Error::Generic(
                "Unable to determine author name. Please set GIT_AUTHOR_NAME environment variable".into()
            )),
        };
        
        let email = match env::var("GIT_AUTHOR_EMAIL") {
            Ok(email) => email,
            Err(_) => format!("{}@{}", name, "localhost"), // Fallback email
        };
        
        let author = Author::new(name, email);
        let mut commit = Commit::new(
            parent.clone(),
            tree_oid.clone(),
            author,
            message.to_string()
        );
        
        if let Err(e) = database.store(&mut commit) {
            return Err(Error::Generic(format!("Failed to store commit: {}", e)));
        }
        
        let commit_oid = commit.get_oid()
            .ok_or(Error::Generic("Commit OID not set after storage".into()))?;
        
        // Update HEAD, following symbolic references if needed
        if let Err(e) = refs.update_head(commit_oid) {
            return Err(Error::Generic(format!("Failed to update HEAD: {}", e)));
        }
        // Numără doar fișierele care s-au schimbat efectiv
        let mut changed_files = 0;
        
        // Dacă există un commit părinte, compară cu el
        if let Some(parent_oid) = &parent {
            // Încarcă commit-ul părinte
            if let Ok(parent_obj) = database.load(parent_oid) {
                if let Some(parent_commit) = parent_obj.as_any().downcast_ref::<Commit>() {
                    let parent_tree_oid = parent_commit.get_tree();
                    
                    // Colectează toate fișierele din commit-ul părinte
                    let mut parent_files = HashMap::<String, String>::new(); // path -> oid
                    Self::collect_files_from_tree(&mut database, &parent_tree_oid, PathBuf::new(), &mut parent_files)?;
                    
                    // Colectează toate fișierele din commit-ul curent
                    let mut current_files = HashMap::<String, String>::new(); // path -> oid
                    Self::collect_files_from_tree(&mut database, &tree_oid, PathBuf::new(), &mut current_files)?;
                    
                    println!("Files in parent commit: {}", parent_files.len());
                    println!("Files in current commit: {}", current_files.len());
                    
                    // Calculează fișierele adăugate (există în curent, nu în părinte)
                    let mut added = 0;
                    for path in current_files.keys() {
                        if !parent_files.contains_key(path) {
                            println!("Added file: {}", path);
                            added += 1;
                        }
                    }
                    
                    // Calculează fișierele șterse (există în părinte, nu în curent)
                    let mut deleted = 0;
                    for path in parent_files.keys() {
                        if !current_files.contains_key(path) {
                            println!("Deleted file: {}", path);
                            deleted += 1;
                        }
                    }
                    
                    // Calculează fișierele modificate (există în ambele, dar OID diferit)
                    let mut modified = 0;
                    for (path, oid) in &current_files {
                        if let Some(parent_oid) = parent_files.get(path) {
                            if parent_oid != oid {
                                println!("Modified file: {}", path);
                                modified += 1;
                            }
                        }
                    }
                    
                    // Numărul total de fișiere schimbate
                    changed_files = added + deleted + modified;
                    println!("Changed files: {} (added: {}, deleted: {}, modified: {})",
                        changed_files, added, deleted, modified);
                }
            }
        } else {
            // Pentru primul commit, toate fișierele sunt noi
            for entry in &database_entries {
                let path = entry.get_name();
                // Să nu numărăm directoarele ca fișiere
                if !path.ends_with('/') {
                    changed_files += 1;
                }
            }
        }
        
        // Print commit message
        let is_root = if parent.is_none() { "(root-commit) " } else { "" };
        let first_line = message.lines().next().unwrap_or("");
        
        let elapsed = start_time.elapsed();
        println!(
            "[{}{}] {} ({:.2}s)", 
            is_root, 
            commit.get_oid().unwrap(), 
            first_line,
            elapsed.as_secs_f32()
        );
        
        // Print a summary of the commit using the correctly counted changed files
        println!(
            "{} file{} changed", 
            changed_files, 
            if changed_files == 1 { "" } else { "s" }
        );
        
        Tree::inspect_tree_structure(&mut database, &tree_oid, 0)?;
        Ok(())
    }
    
    // Funcția de colectare a fișierelor din arbore
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
