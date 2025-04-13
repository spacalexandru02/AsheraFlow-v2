use std::path::{Path, PathBuf};

use crate::errors::error::Error;
use crate::core::workspace::Workspace;
use crate::core::index::index::Index;
use crate::core::database::database::Database;
use crate::core::color::Color;

// Enum pentru statusul verificărilor de ștergere
#[derive(Debug)]
enum RemovalStatus {
    Safe,
    BothChanged,
    Uncommitted,
    Unstaged,
}

pub struct RmCommand;

impl RmCommand {
    pub fn execute(paths: &[String], cached: bool, force: bool, recursive: bool) -> Result<(), Error> {
        let workspace = Workspace::new(Path::new("."));
        let git_path = workspace.root_path.join(".ash");
        let mut database = Database::new(git_path.join("objects"));
        let mut index = Index::new(git_path.join("index"));
        
        // Try to acquire the lock on the index
        if !index.load_for_update()? {
            return Err(Error::Lock(format!(
                "Unable to acquire lock on index. Another process may be using it."
            )));
        }
        
        // Get HEAD OID
        let head_oid = match workspace.read_head() {
            Ok(head) => head,
            Err(_) => {
                // Release the lock if we can't get HEAD - pentru moment, ignorăm eroarea
                //index.release_lock()?;
                return Err(Error::Generic("Failed to read HEAD".to_string()));
            }
        };
        
        // Initialize error tracking
        let mut uncommitted: Vec<PathBuf> = Vec::new();
        let mut unstaged: Vec<PathBuf> = Vec::new();
        let mut both_changed: Vec<PathBuf> = Vec::new();
        
        // Expand and check each path
        let mut expanded_paths: Vec<PathBuf> = Vec::new();
        
        for path_str in paths {
            match Self::expand_path(&index, path_str, recursive) {
                Ok(mut paths) => {
                    expanded_paths.append(&mut paths);
                }
                Err(e) => {
                    // Release the lock on error - pentru moment, ignorăm eroarea
                    //index.release_lock()?;
                    return Err(e);
                }
            }
        }
        
        // Plan removal for each path
        for path in &expanded_paths {
            match Self::plan_removal(&workspace, &mut database, &index, path, &head_oid, force, cached) {
                Ok(result) => {
                    match result {
                        RemovalStatus::BothChanged => both_changed.push(path.clone()),
                        RemovalStatus::Uncommitted => if !cached { uncommitted.push(path.clone()) },
                        RemovalStatus::Unstaged => if !cached { unstaged.push(path.clone()) },
                        RemovalStatus::Safe => {}
                    }
                }
                Err(e) => {
                    // Release the lock on error - pentru moment, ignorăm eroarea
                    //index.release_lock()?;
                    return Err(e);
                }
            }
        }
        
        // Check for errors
        if !both_changed.is_empty() || !uncommitted.is_empty() || !unstaged.is_empty() {
            Self::print_errors(&both_changed, "staged content different from both the file and the HEAD");
            Self::print_errors(&uncommitted, "changes staged in the index");
            Self::print_errors(&unstaged, "local modifications");
            
            // Release the lock - pentru moment, ignorăm eroarea
            //index.release_lock()?;
            return Err(Error::Generic("Cannot remove due to uncommitted changes".to_string()));
        }
        
        // Remove all files
        for path in expanded_paths {
            Self::remove_file(&workspace, &mut index, &path, cached)?;
            println!("rm '{}'", path.display());
        }
        
        // Write index updates
        index.write_updates()?;
        
        Ok(())
    }
    
    // Expand a path (handle directories if -r is specified)
    fn expand_path(index: &Index, path_str: &str, recursive: bool) -> Result<Vec<PathBuf>, Error> {
        let path = PathBuf::from(path_str);
        
        if index.tracked_directory(&path) {
            if recursive {
                // Get all child paths
                return Ok(index.child_paths(&path).iter().map(PathBuf::from).collect());
            } else {
                return Err(Error::Generic(format!(
                    "not removing '{}' recursively without -r", path_str
                )));
            }
        }
        
        if index.tracked_file(&path) {
            return Ok(vec![path]);
        } else {
            return Err(Error::Generic(format!(
                "pathspec '{}' did not match any files", path_str
            )));
        }
    }
    
    // Plan the removal of a file, checking for conflicts
    fn plan_removal(
        workspace: &Workspace, 
        database: &mut Database, 
        index: &Index, 
        path: &Path, 
        head_oid: &str, 
        force: bool, 
        cached: bool
    ) -> Result<RemovalStatus, Error> {
        // Skip checks if force is enabled
        if force {
            return Ok(RemovalStatus::Safe);
        }
        
        // Check if path is a directory and bail
        match workspace.stat_file(path) {
            Ok(stat) => {
                if stat.is_dir() {
                    return Err(Error::Generic(format!(
                        "rm: '{}': Operation not permitted", path.display()
                    )));
                }
            },
            Err(_) => {} // Ignorăm erorile dacă fișierul nu există
        }
        
        // Get the item from HEAD
        let item = Self::load_tree_entry(database, head_oid, path)?;
        
        // Get the item from index
        // Simplificăm cu get_entry direct din Index
        let entry = index.get_entry(&path.to_string_lossy());
        
        // Get the workspace stat
        let stat_result = workspace.stat_file(path);
        
        // Check for staged changes (HEAD vs index)
        let staged_change = Self::compare_tree_to_index(item.as_ref(), entry);
        
        // Check for unstaged changes (index vs workspace)
        let unstaged_change = if stat_result.is_ok() {
            Self::compare_index_to_workspace(entry, stat_result.ok().as_ref())?
        } else {
            None
        };
        
        // Determine status
        if staged_change.is_some() && unstaged_change.is_some() {
            return Ok(RemovalStatus::BothChanged);
        } else if staged_change.is_some() && !cached {
            return Ok(RemovalStatus::Uncommitted);
        } else if unstaged_change.is_some() && !cached {
            return Ok(RemovalStatus::Unstaged);
        }
        
        Ok(RemovalStatus::Safe)
    }
    
    // Remove a file from index and workspace
    fn remove_file(
        workspace: &Workspace, 
        index: &mut Index, 
        path: &Path, 
        cached: bool
    ) -> Result<(), Error> {
        // Remove from index
        index.remove(path)?;
        
        // Remove from workspace unless --cached is used
        if !cached {
            workspace.remove(path)?;
        }
        
        Ok(())
    }
    
    // Print errors for a specific error type
    fn print_errors(paths: &[PathBuf], message: &str) {
        if paths.is_empty() {
            return;
        }
        
        let files_have = if paths.len() == 1 { "file has" } else { "files have" };
        
        println!("{} the following {} {}:", 
            Color::red("error:"), 
            files_have, 
            message
        );
        
        for path in paths {
            println!("    {}", path.display());
        }
    }
    
    // Helper to load a tree entry from a commit - implementare simplificată
    fn load_tree_entry(database: &mut Database, oid: &str, _path: &Path) -> Result<Option<Box<dyn crate::core::database::database::GitObject>>, Error> {
        // Pentru moment, doar verificăm că commit-ul există, fără a căuta obiectul specific
        match database.load(oid) {
            Ok(_) => Ok(None), // Returnăm None pentru orice cale
            Err(e) => Err(e),
        }
    }
    
    // Helper to compare tree entry to index entry - implementare simplificată
    fn compare_tree_to_index(_tree_entry: Option<&Box<dyn crate::core::database::database::GitObject>>, _index_entry: Option<&crate::core::index::entry::Entry>) -> Option<String> {
        // Pentru moment, presupunem că nu sunt diferențe
        None
    }
    
    // Helper to compare index entry to workspace - implementare simplificată
    fn compare_index_to_workspace(_index_entry: Option<&crate::core::index::entry::Entry>, _stat: Option<&std::fs::Metadata>) -> Result<Option<String>, Error> {
        // Pentru moment, presupunem că nu sunt diferențe
        Ok(None)
    }
} 