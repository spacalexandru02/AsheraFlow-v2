// src/commands/merge.rs
use std::time::Instant;
use std::env;
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use crate::errors::error::Error;
use crate::core::merge::inputs::Inputs;
use crate::core::merge::resolve::Resolve;
use crate::core::refs::Refs;
use crate::core::database::database::Database;
use crate::core::database::database::GitObject;
use crate::core::database::commit::Commit;
use crate::core::database::author::Author;
use crate::core::path_filter::PathFilter;
use crate::core::workspace::Workspace;
use crate::core::database::tree::{Tree, TreeEntry};
use crate::core::file_mode::FileMode;
use crate::core::database::entry::DatabaseEntry;


const MERGE_MSG: &str = "\
Merge branch '%s'

# Please enter a commit message to explain why this merge is necessary,
# especially if it merges an updated upstream into a topic branch.
#
# Lines starting with '#' will be ignored, and an empty message aborts
# the commit.
";

pub struct MergeCommand;

impl MergeCommand {
    pub fn execute(revision: &str, message: Option<&str>) -> Result<(), Error> {
        let start_time = Instant::now();

        println!("Merge started...");

        // --- Debug: Print environment details ---
        println!("==== Merge Environment Debug ====");
        match std::env::current_dir() {
            Ok(cwd) => println!("Current directory: {}", cwd.display()),
            Err(e) => println!("Warning: Could not get current directory: {}", e),
        }
         let repo_root_display = ".";
         println!("Workspace root: {}", repo_root_display);
         let git_dir_path = Path::new(repo_root_display).join(".ash");
         println!("Git directory: {}", git_dir_path.display());
         if git_dir_path.exists() {
             println!("  Exists: true");
             if git_dir_path.is_dir() {
                 println!("  Is directory: true");
                 match std::fs::metadata(&git_dir_path) {
                     Ok(meta) => {
                         #[cfg(unix)]
                         {
                             use std::os::unix::fs::PermissionsExt;
                             println!("  Permissions: {:o}", meta.permissions().mode());
                         }
                         #[cfg(not(unix))]
                         {
                             println!("  Permissions: (Windows - check manually)");
                         }
                     }
                     Err(e) => println!("  Warning: Could not get metadata: {}", e),
                 }
             } else {
                 println!("  Is directory: false");
             }
         } else {
             println!("  Exists: false");
         }

         println!("\nContents of current directory:");
         match std::fs::read_dir(".") {
             Ok(entries) => {
                 for entry_result in entries {
                     if let Ok(entry) = entry_result {
                         let path = entry.path();
                         let type_str = if path.is_dir() { "(directory)" } else if path.is_file() { "(file)" } else { "(other)" };
                         println!("  {} {}", path.display(), type_str);
                     }
                 }
             }
             Err(e) => println!("  Warning: Could not read current directory contents: {}", e),
         }
         println!("================================");
         // --- End Debug ---


        // Initialize repository components
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");

        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository (or any of the parent directories): .ash directory not found".into()));
        }

        let workspace = Workspace::new(root_path);
        let mut database = Database::new(git_path.join("objects"));
        let mut index = crate::core::index::index::Index::new(git_path.join("index"));
        let refs = Refs::new(&git_path);

        // --- Lock index EARLY and ensure rollback on ANY error ---
        if !index.load_for_update()? {
             return Err(Error::Lock("Failed to acquire lock on index".to_string()));
        }
        // Use a guard or closure to ensure rollback, or call manually in all error paths
        let result = (|| { // Start closure

            if index.has_conflict() {
                return Err(Error::Generic("Cannot merge with conflicts. Fix conflicts and commit first.".into()));
            }

            let head_oid = match refs.read_head()? {
                Some(oid) => oid,
                None => return Err(Error::Generic("No HEAD commit found. Create an initial commit first.".into())),
            };

            let inputs = Inputs::new(&mut database, &refs, "HEAD".to_string(), revision.to_string())?;

            if inputs.already_merged() {
                println!("Already up to date.");
                return Err(Error::Generic("Already up to date.".into())); // Use error channel for special messages
            }

            if inputs.is_fast_forward() {
                println!("Fast-forward possible.");
                // Pass mutable refs to database and index into fast forward
                return Self::handle_fast_forward(
                    &mut database,
                    &workspace,
                    &mut index,
                    &refs,
                    &inputs.left_oid,
                    &inputs.right_oid
                );
                // NOTE: handle_fast_forward now handles its own index write/commit/rollback
            }

            // --- Recursive Merge ---
             println!("Performing recursive merge.");
            let mut merge_resolver = Resolve::new(&mut database, &workspace, &mut index, &inputs);
            merge_resolver.on_progress = |info| println!("{}", info);

             let merge_result = merge_resolver.execute();

             if let Err(e) = merge_result {
                  if e.to_string().contains("Automatic merge failed") || e.to_string().contains("fix conflicts") {
                       // Write index with conflicts before returning error
                       if !index.write_updates()? {
                           println!("Warning: Index with conflicts was not written (no changes detected by index module).");
                       }
                       return Err(e); // Return conflict error, index lock committed/rolled back by write_updates
                  } else {
                       return Err(e); // Return other resolve errors, index lock released by guard/closure end
                  }
             }

            // --- Merge succeeded without conflicts ---
            if !index.write_updates()? {
                 println!("Warning: Index write reported no changes after successful merge resolution.");
            }


            // --- Commit the successful merge ---
            let commit_message = message.map(|s| s.to_string()).unwrap_or_else(|| {
                format!("Merge branch '{}' into {}", revision, inputs.left_name)
            });
             // Ensure Author details are configured
             let author_name = env::var("GIT_AUTHOR_NAME").unwrap_or_else(|_| {
                 eprintln!("Warning: GIT_AUTHOR_NAME not set. Using default.");
                 "Default Author".to_string()
             });
             let author_email = env::var("GIT_AUTHOR_EMAIL").unwrap_or_else(|_| {
                  eprintln!("Warning: GIT_AUTHOR_EMAIL not set. Using default.");
                  "author@example.com".to_string()
             });
            let author = Author::new(author_name, author_email);


            let tree_oid = Self::write_tree_from_index(&mut database, &index)?; // Pass immutable index now

            let parent1 = head_oid.clone();
            let parent2 = inputs.right_oid.clone();
            let final_message = format!("{}\n\nMerge-Parent: {}", commit_message, parent2); // Simplified parent info

             let mut commit = Commit::new( Some(parent1), tree_oid.clone(), author.clone(), final_message );

             database.store(&mut commit)?;
             let commit_oid = commit.get_oid().cloned().ok_or(Error::Generic("Commit OID not set after storage".into()))?;
             refs.update_head(&commit_oid)?;

             let elapsed = start_time.elapsed();
             println!("Merge completed successfully in {:.2}s", elapsed.as_secs_f32());

            Ok(()) // Success for recursive merge

        })(); // End closure

        // --- Ensure rollback if closure returned error ---
         if result.is_err() {
              // Check specific non-fatal "errors" first
              if let Err(ref e) = result {
                   if e.to_string() == "Already up to date." {
                        index.rollback()?; // Release lock for this case
                        return Ok(()); // Exit successfully
                   }
                   // Conflicts are handled within the closure now, index lock committed/rolled back by write_updates
                   if e.to_string().contains("fix conflicts") {
                       return result; // Return the conflict error
                   }
              }
              // For other errors, ensure rollback
              index.rollback()?;
         }

        result // Return the final result (Ok or Err)
    }


    // --- *** REVISED handle_fast_forward using DIFF approach *** ---
    fn handle_fast_forward(
        database: &mut Database,
        workspace: &Workspace,
        index: &mut crate::core::index::index::Index,
        refs: &Refs,
        current_oid: &str,
        target_oid: &str,
    ) -> Result<(), Error> {
        // Note: index is already locked by the caller (execute)
        let a_short = &current_oid[0..std::cmp::min(8, current_oid.len())];
        let b_short = &target_oid[0..std::cmp::min(8, target_oid.len())];

        println!("Updating {}..{}", a_short, b_short);
        println!("Fast-forward");

        // 1. Get the tree OID for the target commit
        let target_commit_obj = database.load(target_oid)?;
        let target_commit = match target_commit_obj.as_any().downcast_ref::<Commit>() {
            Some(c) => c,
            None => return Err(Error::Generic(format!("Target OID {} is not a commit", target_oid))),
        };
        let target_tree_oid = target_commit.get_tree();
        println!("Target tree OID: {}", target_tree_oid);

        // 2. Get the tree OID for the current commit
        let current_commit_obj = database.load(current_oid)?;
        let current_commit = match current_commit_obj.as_any().downcast_ref::<Commit>() {
            Some(c) => c,
            None => return Err(Error::Generic(format!("Current HEAD OID {} is not a commit", current_oid))),
        };
        let current_tree_oid = current_commit.get_tree();
        println!("Current tree OID: {}", current_tree_oid);

        // 3. Calculate the diff between the current tree and the target tree
        let path_filter = PathFilter::new();
        println!("Calculating tree diff between current ({}) and target ({})", current_tree_oid, target_tree_oid);
        let tree_diff = database.tree_diff(Some(current_tree_oid), Some(target_tree_oid), &path_filter)?;
        println!("Tree diff calculated, {} changes found", tree_diff.len());

        let mut diff_applied = false; // Track if we actually applied changes

        // 4. Apply the changes from the diff to the workspace and index
        if tree_diff.is_empty() {
            println!("No tree changes detected between commits.");
            index.set_changed(false); // No changes to index
        } else {
            for (path, (old_entry, new_entry)) in &tree_diff { // Iterate over reference
                println!("Applying change for: {}", path.display());
                match (old_entry, new_entry) {
                    (Some(_old), Some(new)) => { // Modified
                        if new.get_file_mode().is_directory() {
                            println!("  -> Modified Directory (ensuring exists)");
                            workspace.make_directory(&path)?;
                            
                            // FIXED: Process directory contents explicitly
                            // Load the directory tree and process all files within it
                            let tree_obj = database.load(new.get_oid())?;
                            if let Some(tree) = tree_obj.as_any().downcast_ref::<Tree>() {
                                Self::process_tree_entries(tree, path, database, workspace, index)?;
                            }
                        } else {
                            println!("  -> Modified File");
                            Self::update_workspace_file(database, workspace, index, &path, new.get_oid(), &new.get_file_mode())?;
                        }
                    },
                    (None, Some(new)) => { // Added
                        if new.get_file_mode().is_directory() {
                            println!("  -> Added Directory");
                            workspace.make_directory(&path)?;
                            
                            // FIXED: Process directory contents for newly added directories
                            let tree_obj = database.load(new.get_oid())?;
                            if let Some(tree) = tree_obj.as_any().downcast_ref::<Tree>() {
                                Self::process_tree_entries(tree, path, database, workspace, index)?;
                            }
                        } else {
                            println!("  -> Added File");
                            Self::update_workspace_file(database, workspace, index, &path, new.get_oid(), &new.get_file_mode())?;
                        }
                    },
                    (Some(old), None) => { // Deleted
                        println!("  -> Deleted");
                        let path_str = path.to_string_lossy().to_string();
                        // Check type before removing
                        if old.get_file_mode().is_directory() {
                            println!("  -> Removing directory: {}", path.display());
                            workspace.force_remove_directory(&path)?; // Use force for simplicity
                        } else {
                            println!("  -> Removing file: {}", path.display());
                            workspace.remove_file(&path)?;
                        }
                        index.remove(&PathBuf::from(&path_str))?; // Remove from index
                    },
                    (None, None) => {
                        println!("  -> Warning: Diff entry with no old or new state for {}", path.display());
                    }
                }
            }
            diff_applied = true; // Mark that changes were applied
            index.set_changed(true); // Index was changed
        }

        // 5. Write the updated index
        println!("Attempting to write index updates...");
        match index.write_updates() {
            Ok(updated) => {
                if updated {
                    println!("Index successfully written.");
                } else if !diff_applied {
                    println!("Index write skipped: No changes were applied.");
                    // No rollback needed, index lock will be released by caller
                } else {
                    println!("Warning: Index write reported no changes, but diff was applied.");
                    // Index state might be inconsistent with disk, lock will be released by caller
                }
            },
            Err(e) => {
                println!("ERROR writing index updates: {}", e);
                // Rollback is handled by write_updates on error, caller will handle lock release
                return Err(e);
            }
        }

        // 6. Update HEAD reference
        println!("Attempting to update HEAD to {}", target_oid);
        match refs.update_head(target_oid) {
            Ok(_) => println!("Successfully updated HEAD"),
            Err(e) => {
                println!("ERROR updating HEAD: {}", e);
                // Potentially leave repo in inconsistent state (index updated, HEAD not)
                return Err(e); // Caller will handle rollback
            }
        }

        println!("Fast-forward merge completed.");
        // Index lock is committed by write_updates or rolled back by caller on error
        Ok(())
    }

    // New helper function to process all entries in a tree
    fn process_tree_entries(
        tree: &Tree,
        parent_path: &Path,
        database: &mut Database,
        workspace: &Workspace,
        index: &mut crate::core::index::index::Index
    ) -> Result<(), Error> {
        // 1. Colectează toate intrările din noul arbore
        let mut target_entries = HashMap::new();
        for (name, entry) in tree.get_entries() {
            target_entries.insert(name.clone(), entry.clone());
        }
        
        // 2. Obține lista fișierelor existente din workspace pentru acest director
        let mut current_files = HashSet::new();
        let full_dir_path = workspace.root_path.join(parent_path);
        if full_dir_path.exists() && full_dir_path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&full_dir_path) {
                for entry_result in entries {
                    if let Ok(entry) = entry_result {
                        let file_name = entry.file_name().to_string_lossy().to_string();
                        // Ignoră fișierele ascunse
                        if !file_name.starts_with('.') {
                            current_files.insert(file_name);
                        }
                    }
                }
            }
        }
        
        // 3. Acum procesăm toate intrările - adăugare/modificare
        for (name, entry) in tree.get_entries() {
            let entry_path = parent_path.join(name);
            
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    // E un fișier, scrie-l în workspace
                    println!("  -> Writing file in directory: {}", entry_path.display());
                    Self::update_workspace_file(database, workspace, index, &entry_path, oid, mode)?;
                    // Elimină acest fișier din lista fișierelor existente (l-am procesat deja)
                    current_files.remove(name);
                },
                TreeEntry::Tree(subtree) => {
                    // E un director, asigură-te că există și apoi procesează-l recursiv
                    println!("  -> Processing subdirectory: {}", entry_path.display());
                    workspace.make_directory(&entry_path)?;
                    
                    if let Some(subtree_oid) = subtree.get_oid() {
                        let subtree_obj = database.load(subtree_oid)?;
                        if let Some(subtree) = subtree_obj.as_any().downcast_ref::<Tree>() {
                            Self::process_tree_entries(subtree, &entry_path, database, workspace, index)?;
                        }
                    }
                    // Și aici eliminăm directorul din lista fișierelor existente
                    current_files.remove(name);
                }
            }
        }
        
        // 4. Șterge fișierele care existau înainte dar nu mai există în noul arbore
        for old_name in current_files {
            let old_path = parent_path.join(&old_name);
            let path_str = old_path.to_string_lossy().to_string();
            
            println!("  -> Removing file not in target tree: {}", old_path.display());
            
            // Verifică dacă e director sau fișier
            let full_path = workspace.root_path.join(&old_path);
            if full_path.is_dir() {
                workspace.force_remove_directory(&old_path)?;
            } else {
                workspace.remove_file(&old_path)?;
            }
            
            // Șterge din index
            index.remove(&PathBuf::from(&path_str))?;
        }
        
        Ok(())
    }

    // Helper to update a single file
    fn update_workspace_file(
        database: &mut Database,
        workspace: &Workspace,
        index: &mut crate::core::index::index::Index,
        path: &PathBuf,
        oid: &str,
        mode: &FileMode,
    ) -> Result<(), Error> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let parent_full_path = workspace.root_path.join(parent);
                if !parent_full_path.exists() {
                    workspace.make_directory(parent)?;
                }
            }
        }
        let blob_obj = database.load(oid)?;
        let content = blob_obj.to_bytes();
        workspace.write_file(&path, &content)?;
        let stat = workspace.stat_file(&path)?;
        index.add(&path, oid, &stat)?;
        Ok(())
    }

    // --- write_tree_from_index - Takes immutable index ---
    fn write_tree_from_index(database: &mut Database, index: &crate::core::index::index::Index) -> Result<String, Error> {
        let database_entries: Vec<_> = index.each_entry()
            .filter(|entry| entry.stage == 0) // Only include stage 0 entries
            .map(|index_entry| {
                DatabaseEntry::new(
                    index_entry.get_path().to_string(),
                    index_entry.get_oid().to_string(),
                    &index_entry.mode_octal()
                )
            })
            .collect();

         if database_entries.is_empty() {
              let mut empty_tree = Tree::new();
              database.store(&mut empty_tree)?;
              return empty_tree.get_oid().cloned().ok_or_else(|| Error::Generic("Failed to get OID for empty tree".into()));
         }

        let mut root = crate::core::database::tree::Tree::build(database_entries.iter())?;
        root.traverse(|tree| database.store(tree).map(|_| ()))?;
        let tree_oid = root.get_oid()
            .ok_or(Error::Generic("Tree OID not set after storage".into()))?;
        Ok(tree_oid.clone())
    }
}