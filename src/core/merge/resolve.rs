// src/core/merge/resolve.rs
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::core::database::blob::Blob;
use crate::core::database::database::{Database, GitObject};
use crate::core::database::entry::DatabaseEntry;
use crate::core::file_mode::FileMode;
use crate::core::index::index::Index;
use crate::core::workspace::Workspace;
use crate::errors::error::Error;
use crate::core::merge::diff3;
use crate::core::merge::inputs::MergeInputs;
use crate::core::path_filter::PathFilter;

pub struct Resolve<'a, T: MergeInputs> {
    database: &'a mut Database,
    workspace: &'a Workspace,
    index: &'a mut Index,
    inputs: &'a T,
    left_diff: HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>,
    right_diff: HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>,
    clean_diff: HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>,
    conflicts: HashMap<String, Vec<Option<DatabaseEntry>>>,
    untracked: HashMap<String, DatabaseEntry>, // For renamed files in conflicts
    pub on_progress: fn(String),
}

impl<'a, T: MergeInputs> Resolve<'a, T> {
    pub fn new(
        database: &'a mut Database,
        workspace: &'a Workspace,
        index: &'a mut Index,
        inputs: &'a T,
    ) -> Self {
        Self {
            database,
            workspace,
            index,
            inputs,
            left_diff: HashMap::new(),
            right_diff: HashMap::new(),
            clean_diff: HashMap::new(),
            conflicts: HashMap::new(),
            untracked: HashMap::new(),
            on_progress: |_info| (),
        }
    }

     // Main execution logic for recursive merge
     pub fn execute(&mut self) -> Result<(), Error> {
         println!("Executing merge resolution");

         // Prepare the tree differences and identify conflicts
         self.prepare_tree_diffs()?; // Populates self.conflicts and self.untracked

         // Write untracked files first (renamed conflict files) to prevent data loss
         self.write_untracked_files()?;

         // Apply clean (non-conflicting) changes to workspace and index
         self.apply_clean_changes()?;

         // Add conflict information (stages 1, 2, 3) to the index
         self.add_conflicts_to_index(); // This marks index.changed = true if conflicts exist

         // Check if conflicts were detected
         if !self.conflicts.is_empty() {
             println!("Found {} conflicts.", self.conflicts.len());
             // Return error indicating conflicts, index lock is kept by caller (main.rs)
             // because index.write_updates() will be called there to save conflict state.
             return Err(Error::Generic("Automatic merge failed; fix conflicts and then commit the result.".into()));
         }

         // No conflicts were found during preparation and resolution
         println!("Merge resolved successfully with no conflicts.");
         Ok(()) // Index lock released by caller (main.rs) via index.write_updates()
     }


    fn file_dir_conflict(
        &mut self,
        path: &Path,
        diff: &HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>,
        name: &str, // Branch name where the file exists (the other has the directory)
    ) {
        // This logic is now primarily handled within same_path_conflict and record_parent_dir_conflict
        // Keeping the shell here, but it might be redundant or need adjustment if specific
        // parent-based file/dir conflicts need different handling than direct ones.

        let path_str = path.to_string_lossy().to_string();
        println!("Checking legacy file/dir parent conflict for: {}", path_str);

        // Consider if this loop logic is still needed or if the direct check + parent check in prepare_tree_diffs is sufficient.
        // For now, let's keep it but be aware it might double-log or conflict with other checks.

        // Example: Check parent directories (original logic)
        /*
        for parent in self.parent_directories(path) {
            if let Some((old_item, new_item_opt)) = diff.get(&parent) {
                 if let Some(new_item) = new_item_opt {
                      // If parent is a FILE in the other diff map
                      if !new_item.get_file_mode().is_directory() {
                           let parent_path = parent.to_string_lossy().to_string();
                           println!("Found parent file/dir conflict at: {}", parent_path);
                           // ... rest of conflict recording logic ...
                           break; // Stop checking higher parents
                      }
                 }
            }
        }
        */
    }


    fn apply_clean_changes(&mut self) -> Result<(), Error> {
        println!("Applying {} clean changes...", self.clean_diff.len());
        let clean_diff_clone = self.clean_diff.clone(); // Clone to allow mutable borrow of self later
        for (path, (_, new_entry_opt)) in clean_diff_clone { // Iterate over the clone
            println!("  Applying change for: {}", path.display());
            if let Some(new_entry) = new_entry_opt {
                if !new_entry.get_file_mode().is_directory() {
                    println!("    Updating file...");
                    // Call helper method using self
                    self.update_workspace_file(&path, new_entry.get_oid(), &new_entry.get_file_mode())?;
                } else {
                    println!("    Ensuring directory exists...");
                    self.workspace.make_directory(&path)?;
                    // Optionally add directory to index if needed
                    // let stat = self.workspace.stat_file(&path)?;
                    // self.index.add(&path, new_entry.get_oid(), &stat)?;
                }
            } else {
                // Entry is None, meaning deletion
                println!("    Deleting path...");
                let path_str = path.to_string_lossy().to_string();
                let full_path = self.workspace.root_path.join(&path); // Use full path for checks
                if full_path.exists() {
                     if full_path.is_dir() {
                          self.workspace.force_remove_directory(&path)?;
                     } else {
                          self.workspace.remove_file(&path)?;
                     }
                } else {
                    println!("    Path {} already removed.", path.display());
                }
                self.index.remove(&path_str)?;
            }
        }
        println!("Finished applying clean changes.");
        Ok(())
    }


    fn add_conflicts_to_index(&mut self) {
         if self.conflicts.is_empty() { return; }
         println!("Adding {} conflict entries to index...", self.conflicts.len());
        for (path, entries) in &self.conflicts {
             println!("  Adding conflict for: {}", path);
            let path_obj = Path::new(path);
            self.index.add_conflict(path_obj, entries.clone()); // Clones Option<DatabaseEntry>
        }
    }

    fn write_untracked_files(&mut self) -> Result<(), Error> {
        if self.untracked.is_empty() { return Ok(()); }
        println!("Writing {} untracked files resulting from conflicts...", self.untracked.len());
        for (path_str, entry) in &self.untracked {
             println!("  Writing untracked file: {} (OID: {})", path_str, entry.get_oid());
             let blob_obj = self.database.load(entry.get_oid())?;
             let content = blob_obj.to_bytes();
             let path_obj = Path::new(path_str);
             if let Some(parent) = path_obj.parent() {
                  if !parent.as_os_str().is_empty() {
                      self.workspace.make_directory(parent)?;
                  }
             }
            self.workspace.write_file(path_obj, &content)?;
        }
        println!("Successfully wrote all untracked files.");
        Ok(())
    }


    fn parent_directories(&self, path: &Path) -> Vec<PathBuf> {
        let mut result = Vec::new();
        let mut current = PathBuf::from(path);
        while let Some(parent) = current.parent() {
            if parent.as_os_str().is_empty() { break; }
            result.push(parent.to_path_buf());
            current = parent.to_path_buf();
        }
        result
    }

    fn log(&self, message: String) {
        (self.on_progress)(message);
    }

    // --- Logging functions ---
    fn log_conflict(&self, path: &Path, rename: Option<String>) {
        let path_str = path.to_string_lossy().to_string();
        if let Some(conflict) = self.conflicts.get(&path_str) {
            let base = conflict[0].clone();
            let left = conflict[1].clone();
            let right = conflict[2].clone();

            if left.is_some() && right.is_some() { self.log_left_right_conflict(&path_str); }
            else if base.is_some() && (left.is_some() || right.is_some()) { self.log_modify_delete_conflict(&path_str, rename); }
            else if let Some(renamed_to) = rename { self.log_file_directory_conflict(&path_str, renamed_to); }
            else { /* Handle cases with no rename? Might be file/dir conflict logged differently */ }
        } else {
             self.log(format!("CONFLICT: Merge conflict detected for {}", path_str));
        }
    }
    fn log_left_right_conflict(&self, path: &str) {
         if let Some(conflict) = self.conflicts.get(path) {
             let base = conflict[0].clone();
            let conflict_type = if base.is_some() { "content" } else { "add/add" };
             self.log(format!("CONFLICT ({}): Merge conflict in {}", conflict_type, path));
         } else { self.log(format!("CONFLICT (unknown type): Merge conflict in {}", path)); }
    }
    fn log_modify_delete_conflict(&self, path: &str, rename: Option<String>) {
        let (deleted, modified) = self.log_branch_names(path);
        let rename_msg = rename.map_or(String::new(), |r| format!(" at {}", r));
        self.log(format!( "CONFLICT (modify/delete): {} deleted in {} and modified in {}. Version {} of {} left in tree{}.", path, deleted, modified, modified, path, rename_msg, ));
    }
    fn log_file_directory_conflict(&self, path: &str, rename: String) {
        let conflict_type = if let Some(conflict) = self.conflicts.get(path) {
             if conflict[1].is_some() { "file/directory" } else { "directory/file" }
        } else { "unknown" };
        let (branch, _) = self.log_branch_names(path);
        self.log(format!( "CONFLICT ({}): There is a directory with name {} in {}. Adding {} as {}", conflict_type, path, branch, path, rename, ));
    }
    fn log_branch_names(&self, path: &str) -> (String, String) {
        let (a, b) = (self.inputs.left_name(), self.inputs.right_name());
         if let Some(conflict) = self.conflicts.get(path) {
             if conflict[1].is_some() { (b.clone(), a.clone()) } else { (a.clone(), b.clone()) }
         } else { (a.clone(), b.clone()) }
    }
    // --- End Logging Functions ---


    fn merge_blobs(
        &mut self,
        base_oid: Option<&str>,
        left_oid: Option<&str>,
        right_oid: Option<&str>,
    ) -> Result<(bool, String), Error> {
        if let Some(result) = Resolve::<T>::merge3_oid(base_oid, left_oid, right_oid) {
            return Ok((true, result.to_string()));
        }
        let blobs: Vec<String> = vec![base_oid, left_oid, right_oid]
            .into_iter()
            .map(|oid| -> Result<String, Error> {
                if let Some(oid_str) = oid {
                     if oid_str.len() == 40 && oid_str.chars().all(|c| c.is_ascii_hexdigit()) {
                         let blob_obj = self.database.load(oid_str)?;
                         let content = blob_obj.to_bytes();
                         Ok(String::from_utf8_lossy(&content).to_string())
                     } else { Ok(String::new()) }
                } else { Ok(String::new()) }
            })
            .collect::<Result<Vec<String>, Error>>()?;

        let merge_result = diff3::merge(&blobs[0], &blobs[1], &blobs[2])?;
        let result_text = merge_result.to_string( Some(&self.inputs.left_name()), Some(&self.inputs.right_name()), );
        let mut blob = Blob::new(result_text.as_bytes().to_vec());
        self.database.store(&mut blob)?;
        let blob_oid = blob.get_oid().map(|s| s.to_string()).unwrap_or_default();
        Ok((merge_result.is_clean(), blob_oid))
    }

    fn merge_modes(
        &self,
        base_mode: Option<FileMode>,
        left_mode: Option<FileMode>,
        right_mode: Option<FileMode>,
    ) -> (bool, FileMode) {
        if left_mode == base_mode || left_mode == right_mode { return (true, right_mode.unwrap_or(FileMode::REGULAR)); }
        if right_mode == base_mode { return (true, left_mode.unwrap_or(FileMode::REGULAR)); }
        if left_mode.is_none() { return (right_mode.is_none(), right_mode.unwrap_or(FileMode::REGULAR)); }
        if right_mode.is_none() { return (false, left_mode.unwrap_or(FileMode::REGULAR)); }
        (false, left_mode.unwrap_or(FileMode::REGULAR))
    }

    // Associated function, no `self`
    fn merge3_oid<'b>( base: Option<&'b str>, left: Option<&'b str>, right: Option<&'b str>, ) -> Option<&'b str> {
         if left == right { return left; }
         if left == base { return right; }
         if right == base { return left; }
         None
    }


    fn prepare_tree_diffs(&mut self) -> Result<(), Error> {
        println!("Preparing tree diffs for merge");
        let base_oids = self.inputs.base_oids();
        let base_oid_opt = base_oids.first().map(String::as_str);
        let path_filter = PathFilter::new();

        self.left_diff = self.database.tree_diff( base_oid_opt, Some(&self.inputs.left_oid()), &path_filter, )?;
        println!("Left diff ({} vs Base) has {} entries", self.inputs.left_name(), self.left_diff.len());

        self.right_diff = self.database.tree_diff( base_oid_opt, Some(&self.inputs.right_oid()), &path_filter, )?;
        println!("Right diff ({} vs Base) has {} entries", self.inputs.right_name(), self.right_diff.len());

        self.clean_diff = HashMap::new();
        self.conflicts = HashMap::new();
        self.untracked = HashMap::new();

        let mut all_paths = HashSet::new();
        all_paths.extend(self.left_diff.keys().cloned());
        all_paths.extend(self.right_diff.keys().cloned());

        let paths_to_process: Vec<PathBuf> = all_paths.into_iter().collect();
        println!("Processing {} unique paths", paths_to_process.len());

        for path in paths_to_process {
             println!("Processing path: {}", path.display());

             // Clone entries needed for same_path_conflict and potential later use
             let base_entry = self.left_diff.get(&path).and_then(|(old, _)| old.clone())
                 .or_else(|| self.right_diff.get(&path).and_then(|(old, _)| old.clone()));
             let left_entry = self.left_diff.get(&path).and_then(|(_, new)| new.clone());
             let right_entry = self.right_diff.get(&path).and_then(|(_, new)| new.clone());

             // Extract booleans needed for parent checks *before* potentially moving entries
             let left_new_is_some = left_entry.is_some();
             let left_new_is_dir = left_entry.as_ref().map_or(false, |e| e.get_file_mode().is_directory());
             let right_entry_exists = right_entry.is_some();
             let right_new_is_dir = right_entry.as_ref().map_or(false, |e| e.get_file_mode().is_directory());

             // Call same_path_conflict - it now takes owned Options
             self.same_path_conflict(&path, base_entry, left_entry, right_entry)?;

             // Check parent conflicts only if the current path wasn't already marked as conflicted
              if !self.conflicts.contains_key(path.to_string_lossy().as_ref()) {
                 let mut conflict_found_parent = false;
                 if left_new_is_some && !left_new_is_dir {
                     if let Some(conflicting_parent) = self.check_parent_dir_conflict(&path, &self.right_diff, &self.inputs.right_name()) {
                         self.record_parent_dir_conflict(&path, &conflicting_parent, &self.inputs.right_name());
                         conflict_found_parent = true;
                     }
                 }
                 if !conflict_found_parent && right_entry_exists && !right_new_is_dir {
                     if let Some(conflicting_parent) = self.check_parent_dir_conflict(&path, &self.left_diff, &self.inputs.left_name()) {
                         self.record_parent_dir_conflict(&path, &conflicting_parent, &self.inputs.left_name());
                     }
                 }
              } else {
                   println!("  Skipping parent conflict check for already conflicted path: {}", path.display());
              }
        }

        println!("Tree diff processing complete:");
        println!("  Clean changes: {}", self.clean_diff.len());
        println!("  Conflicts: {}", self.conflicts.len());
        println!("  Untracked files: {}", self.untracked.len());
        Ok(())
    }

    // Takes &self because logging helpers might need self.inputs
     fn check_parent_dir_conflict(
         &self,
         file_path: &Path,
         other_diff: &HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>,
         _other_branch_name: &str,
     ) -> Option<PathBuf> {
         let mut current = file_path.parent();
         while let Some(parent_dir) = current {
             if parent_dir.as_os_str().is_empty() { break; }
             if let Some((_, other_parent_change)) = other_diff.get(parent_dir) {
                 // Check if parent in other diff is a FILE (conflicts with dir structure needed for file_path)
                  if let Some(other_entry) = other_parent_change {
                     if !other_entry.get_file_mode().is_directory() {
                          return Some(parent_dir.to_path_buf()); // Conflict found
                     }
                  }
             }
             current = parent_dir.parent();
         }
         None // No conflict found
     }

     // Takes &mut self to modify conflicts, clean_diff, untracked
      fn record_parent_dir_conflict( &mut self, file_path: &Path, conflicting_parent: &Path, other_branch_name: &str, ) {
          let file_path_str = file_path.to_string_lossy().to_string();
          let parent_path_str = conflicting_parent.to_string_lossy().to_string();

          // Record conflict for the *parent path* where the file/dir type mismatch occurs
          if !self.conflicts.contains_key(&parent_path_str) {
               self.log(format!( "CONFLICT (directory/file): Path conflict between directory '{}' needed for '{}' and file '{}' in branch '{}'", parent_path_str, file_path_str, parent_path_str, other_branch_name ));

              // Retrieve the conflicting entries for the parent path
              let parent_base = self.left_diff.get(conflicting_parent).and_then(|(b, _)| b.clone())
                               .or_else(|| self.right_diff.get(conflicting_parent).and_then(|(b, _)| b.clone()));
               // Determine which diff map corresponds to 'other_branch' to get the file entry
               let other_diff_map = if other_branch_name == self.inputs.left_name() { &self.left_diff } else { &self.right_diff };
               let file_entry = other_diff_map.get(conflicting_parent).and_then(|(_, f)| f.clone()); // The file entry from other branch

               // Determine the directory entry (implicitly exists in current branch structure) - represent as None?
               let dir_entry = None; // Or potentially load the dir entry if needed for stage 3

               // Conflict vec [base, left_entry, right_entry]
                let conflict_vec = if other_branch_name == self.inputs.left_name() {
                     vec![parent_base, file_entry.clone(), dir_entry] // Left has file, Right has dir
                } else {
                     vec![parent_base, dir_entry, file_entry.clone()] // Left has dir, Right has file
                };

              self.conflicts.insert(parent_path_str.clone(), conflict_vec);
              self.clean_diff.remove(conflicting_parent); // Remove parent from clean changes

              // Rename the conflicting file from the other branch
              if let Some(entry) = file_entry {
                   let rename = format!("{}~{}", parent_path_str, other_branch_name);
                   self.untracked.insert(rename.clone(), entry);
                   self.log(format!("Adding conflicting file {} as {}", parent_path_str, rename));
              }

              // Also mark the original file_path as conflicted to prevent applying its changes?
              // This might be too aggressive, depends on desired handling. For now, just conflict the parent.
              // self.conflicts.insert(file_path_str.clone(), vec![...]);
              // self.clean_diff.remove(file_path);
          }
      }


    // Takes &mut self to modify conflicts, untracked, clean_diff
    fn handle_file_directory_conflict(
        &mut self,
        path: &Path,
        file_entry: DatabaseEntry, // The entry that is the file
        branch_with_file: &str // The name of the branch where path is a file
    ) -> Result<(), Error> {
        let path_str = path.to_string_lossy().to_string();
        println!("Handling direct file/directory conflict for {}", path_str);

        if self.conflicts.contains_key(&path_str) { return Ok(()); } // Avoid double recording

        // Base is likely None or the entry from the common ancestor if it existed
        // We need the base entry for the conflict vector if available
         let base_entry = self.left_diff.get(path).and_then(|(old, _)| old.clone())
             .or_else(|| self.right_diff.get(path).and_then(|(old, _)| old.clone()));

        // Determine left/right based on which branch has the file
        let (left_entry, right_entry) = if branch_with_file == self.inputs.left_name() {
            (Some(file_entry.clone()), None) // Left has file, Right implicitly has dir
        } else {
            (None, Some(file_entry.clone())) // Left implicitly has dir, Right has file
        };

        self.conflicts.insert(path_str.clone(), vec![base_entry, left_entry, right_entry]);
        self.clean_diff.remove(path);

        let rename_path = format!("{}~{}", path_str, branch_with_file);
        println!("  Creating renamed file: {}", rename_path);
        self.untracked.insert(rename_path.clone(), file_entry);

        self.log(format!( "CONFLICT (file/directory): '{}' is a file in branch '{}' and a directory in the other.", path_str, branch_with_file ));
        self.log(format!("  Renaming file version to {} to avoid data loss.", rename_path));
        Ok(())
    }

    // Takes &mut self
     fn same_path_conflict(
         &mut self,
         path: &Path,
         base: Option<DatabaseEntry>,
         left: Option<DatabaseEntry>,
         right: Option<DatabaseEntry>,
     ) -> Result<(), Error> {
         let path_str = path.to_string_lossy().to_string();

         if self.conflicts.contains_key(&path_str) { return Ok(()); }

         let left_is_dir = left.as_ref().map_or(false, |e| e.get_file_mode().is_directory());
         let right_is_dir = right.as_ref().map_or(false, |e| e.get_file_mode().is_directory());

          // Handle direct file/directory conflict using the dedicated helper
          if left.is_some() && right.is_some() && left_is_dir != right_is_dir {
               let (file_entry, branch_with_file) = if left_is_dir {
                    (right.clone().unwrap(), self.inputs.right_name()) // Right has file
               } else {
                    (left.clone().unwrap(), self.inputs.left_name()) // Left has file
               };
               // Call the helper, passing the FILE entry and the branch it came from
               self.handle_file_directory_conflict(path, file_entry, &branch_with_file)?;
               return Ok(()); // Conflict handled
          }


         if left == right {
              // Use as_ref for comparison if base is also Option
              if left.is_some() && left.as_ref() != base.as_ref() {
                  self.clean_diff.insert(path.to_path_buf(), (base.clone(), left.clone()));
              }
             return Ok(());
         }

         let base_oid_str = base.as_ref().map(|b| b.get_oid());
         let left_oid_str = left.as_ref().map(|l| l.get_oid());
         let right_oid_str = right.as_ref().map(|r| r.get_oid());

         let base_mode = base.as_ref().map(|b| b.get_file_mode());
         let left_mode = left.as_ref().map(|l| l.get_file_mode());
         let right_mode = right.as_ref().map(|r| r.get_file_mode());


         if left.is_some() && right.is_some() && left != base && right != base && left != right {
              if !left_is_dir && !right_is_dir { self.log(format!("Auto-merging {}", path_str)); }
         }

         let (mode_ok, merged_mode) = self.merge_modes(base_mode, left_mode, right_mode);

         let (oid_ok, merged_oid_str_result) = if left_is_dir || right_is_dir {
              let merged_oid = Resolve::<T>::merge3_oid(base_oid_str, left_oid_str, right_oid_str); // Use Turbofish
              if let Some(oid) = merged_oid { (true, oid.to_string()) }
              else { (false, left_oid_str.unwrap_or("").to_string()) } // Conflict
         } else {
              self.merge_blobs(base_oid_str, left_oid_str, right_oid_str)?
         };

         let merged_entry = if left.is_some() || right.is_some() {
              if !merged_oid_str_result.is_empty() { Some(DatabaseEntry::new( path_str.clone(), merged_oid_str_result.clone(), &merged_mode.to_octal_string(), )) }
              else { None }
         } else { None };

         if merged_entry.as_ref() != base.as_ref() {
              self.clean_diff.insert( path.to_path_buf(), (base.clone(), merged_entry.clone()), );
         }


         if !oid_ok || !mode_ok {
              // Use owned values passed into function for conflict vec
              self.conflicts.insert(path_str.clone(), vec![base, left.clone(), right.clone()]);
              // Check existence using owned values
              let right_entry_exists = right.is_some();
              let rename_path_opt = if left_is_dir && right_entry_exists { Some(format!("{}~{}", path_str, self.inputs.right_name())) }
                                   else if right_is_dir && left.is_some() { Some(format!("{}~{}", path_str, self.inputs.left_name())) }
                                   else { None };
              self.log_conflict(path, rename_path_opt);
         }

         Ok(())
     }


    // Takes &mut self
      fn update_workspace_file(
          &mut self,
          path: &PathBuf,
          oid: &str,
          mode: &FileMode,
      ) -> Result<(), Error> {
          if let Some(parent) = path.parent() {
              if !parent.as_os_str().is_empty() {
                   let parent_full_path = self.workspace.root_path.join(parent);
                   if !parent_full_path.exists() {
                        self.workspace.make_directory(parent)?;
                   }
              }
          }
          let blob_obj = self.database.load(oid)?;
          let content = blob_obj.to_bytes();
          self.workspace.write_file(&path, &content)?;
          let stat = self.workspace.stat_file(&path)?;
          self.index.add(&path, oid, &stat)?;
          Ok(())
      }

} 