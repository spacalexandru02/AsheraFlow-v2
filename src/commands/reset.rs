// src/commands/reset.rs
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::collections::HashMap;
use std::fs;

use crate::errors::error::Error;
use crate::core::workspace::Workspace;
use crate::core::index::index::Index;
use crate::core::database::database::Database;
use crate::core::revision::Revision;
use crate::core::repository::repository::Repository;
use crate::core::refs::Refs;
use crate::core::database::tree::TreeEntry;
use crate::core::database::commit::Commit;
use crate::core::file_mode::FileMode;
use crate::core::database::entry::DatabaseEntry;

// Constanta pentru ORIG_HEAD
pub const ORIG_HEAD: &str = "ORIG_HEAD";
pub const COMMIT_EDITMSG: &str = "COMMIT_EDITMSG";

// Enum pentru modurile de reset
enum Mode {
    Soft,
    Mixed,
    Hard,
}

pub struct ResetCommand;

impl ResetCommand {
    pub fn execute(paths: &[String], soft: bool, mixed: bool, hard: bool, force: bool, reuse_message: Option<&str>) -> Result<(), Error> {
        let start_time = Instant::now();
        println!("Reset started...");
        
        // Inițializare repository
        let mut repo = Repository::new(".")?;
        
        // Citește starea curentă head
        let head_oid = match repo.refs.read_head()? {
            Some(oid) => oid,
            None => return Err(Error::Generic("Fatal: Not a valid object name: HEAD".to_string()))
        };
        
        // Determinăm modul și ținta de reset
        let mode = if hard {
            Mode::Hard
        } else if soft {
            Mode::Soft
        } else {
            Mode::Mixed
        };
        
        // Stabilim commit-ul de resetare
        let mut commit_oid = head_oid.clone();
        let mut remaining_paths = paths.to_vec();
        
        // Verificăm primul argument pentru a vedea dacă este o revizie
        if let Some(first_arg) = paths.get(0) {
            let mut revision = Revision::new(&mut repo, first_arg);
            match revision.resolve("commit") {
                Ok(oid) => {
                    commit_oid = oid;
                    remaining_paths.remove(0); // Îndepărtăm primul argument, rămân doar căile
                },
                Err(_) => {
                    // Nu este o revizie validă, o tratăm ca pe o cale de fișier
                }
            }
        }
        
        // Încărcăm indexul pentru actualizare
        repo.index.load_for_update()?;
        
        // Procesăm resetarea în funcție de mod
        match mode {
            Mode::Soft => {
                // Soft mode: doar actualizează HEAD
                if remaining_paths.is_empty() {
                    // Salvăm HEAD curent în ORIG_HEAD
                    if let Some(old_oid) = repo.refs.read_head()? {
                        let orig_head_path = repo.path.join(".ash").join(ORIG_HEAD);
                        std::fs::write(orig_head_path, format!("{}\n", old_oid))
                            .map_err(|e| Error::Generic(format!("Could not write ORIG_HEAD: {}", e)))?;
                        
                        // If reuse_message is specified, save the commit message to COMMIT_EDITMSG
                        if let Some(rev) = reuse_message {
                            Self::save_commit_message_for_reuse(&mut repo, rev)?;
                        } else {
                            // Otherwise, save the current HEAD's message
                            Self::save_commit_message_for_reuse(&mut repo, "HEAD")?;
                        }
                    }
                    
                    // Actualizăm HEAD
                    repo.refs.update_head(&commit_oid)?;
                    println!("HEAD is now at {}", Self::short_oid(&commit_oid));
                    println!("Commit message saved for reuse");
                } else {
                    return Err(Error::Generic("Cannot do path reset with --soft".to_string()));
                }
            },
            Mode::Mixed => {
                // Mixed mode: actualizează HEAD și indexul
                if remaining_paths.is_empty() {
                    // Salvăm HEAD curent în ORIG_HEAD
                    if let Some(old_oid) = repo.refs.read_head()? {
                        let orig_head_path = repo.path.join(".ash").join(ORIG_HEAD);
                        std::fs::write(orig_head_path, format!("{}\n", old_oid))
                            .map_err(|e| Error::Generic(format!("Could not write ORIG_HEAD: {}", e)))?;
                    }
                    
                    // Resetează întregul index
                    repo.index.clear();
                    Self::reset_tree(&mut repo, &commit_oid, None)?;
                    
                    // Actualizează HEAD
                    repo.refs.update_head(&commit_oid)?;
                    println!("HEAD is now at {}", Self::short_oid(&commit_oid));
                    println!("Index reset to {}", Self::short_oid(&commit_oid));
                } else {
                    // Resetează doar căile specificate
                    for path_str in &remaining_paths {
                        let path = PathBuf::from(path_str);
                        Self::reset_path(&mut repo, &commit_oid, &path)?;
                    }
                    println!("Paths have been reset in the index");
                }
            },
            Mode::Hard => {
                // Hard mode: resetare completă (HEAD, index și workspace)
                if remaining_paths.is_empty() {
                    // Salvăm HEAD curent în ORIG_HEAD
                    if let Some(old_oid) = repo.refs.read_head()? {
                        let orig_head_path = repo.path.join(".ash").join(ORIG_HEAD);
                        std::fs::write(orig_head_path, format!("{}\n", old_oid))
                            .map_err(|e| Error::Generic(format!("Could not write ORIG_HEAD: {}", e)))?;
                    }
                    
                    // Facem hard reset utilizând tree diff, folosind parametrul force
                    Self::hard_reset(&mut repo, &commit_oid, force)?;
                    
                    // Actualizează HEAD
                    repo.refs.update_head(&commit_oid)?;
                    println!("HEAD is now at {}", Self::short_oid(&commit_oid));
                    println!("Index and workspace reset to {}", Self::short_oid(&commit_oid));
                } else {
                    return Err(Error::Generic("Cannot do path reset with --hard".to_string()));
                }
            }
        }
        
        // Scriem modificările indexului
        repo.index.write_updates()?;
        
        let elapsed = start_time.elapsed();
        println!("Reset completed in {:.2}s", elapsed.as_secs_f32());
        
        Ok(())
    }
    
    // Helper to save commit message for reuse
    fn save_commit_message_for_reuse(repo: &mut Repository, revision: &str) -> Result<(), Error> {
        // Parse the revision to get the commit ID
        let commit_oid = if revision == "HEAD" {
            match repo.refs.read_head()? {
                Some(oid) => oid,
                None => return Err(Error::Generic("Fatal: Not a valid object name: HEAD".to_string()))
            }
        } else if revision == "ORIG_HEAD" {
            // Try to read from ORIG_HEAD file
            let orig_head_path = repo.path.join(".ash").join(ORIG_HEAD);
            if !orig_head_path.exists() {
                return Err(Error::Generic("ORIG_HEAD not found".to_string()));
            }
            fs::read_to_string(&orig_head_path)
                .map_err(|e| Error::Generic(format!("Failed to read ORIG_HEAD: {}", e)))?
                .trim()
                .to_string()
        } else {
            // Try to resolve the revision
            let mut revision_parser = Revision::new(repo, revision);
            revision_parser.resolve("commit")?
        };
        
        // Load the commit
        let commit_obj = repo.database.load(&commit_oid)?;
        
        // Get the commit message
        let message = if let Some(commit) = commit_obj.as_any().downcast_ref::<Commit>() {
            commit.get_message().to_string()
        } else {
            return Err(Error::Generic(format!("Object {} is not a commit", commit_oid)));
        };
        
        // Save the message to COMMIT_EDITMSG file
        let edit_msg_path = repo.path.join(".ash").join(COMMIT_EDITMSG);
        fs::write(&edit_msg_path, message)
            .map_err(|e| Error::Generic(format!("Failed to write commit message: {}", e)))?;
        
        Ok(())
    }
    
    // Resetează un arbore întreg sau o cale specifică la starea din commit
    fn reset_tree(repo: &mut Repository, commit_oid: &str, pathname: Option<&Path>) -> Result<(), Error> {
        // Încarcă arborele din commit
        let commit_obj = repo.database.load(commit_oid)?;
        
        // Convertim obiectul la Commit pentru a putea accesa tree_oid
        let commit = if let Some(c) = commit_obj.as_any().downcast_ref::<Commit>() {
            c
        } else {
            return Err(Error::Generic(format!("Object {} is not a commit", commit_oid)));
        };
        
        let tree_oid = commit.get_tree();
        
        // Șterge intrările existente din index pentru calea specificată
        if let Some(path) = pathname {
            repo.index.remove(path)?;
        } else {
            // Dacă nu este specificată nicio cale, resetăm întregul index
            repo.index.clear();
        }
        
        // Încarcă arborele
        let tree_obj = repo.database.load(tree_oid)?;
        
        // Adaugă intrările din arbore în index
        if let Some(path) = pathname {
            // Pentru o cale specifică, adăugăm doar fișierele de sub acea cale
            Self::add_tree_to_index(repo, &tree_obj, path)?;
        } else {
            // Pentru întregul index, adăugăm recursiv toate intrările
            Self::add_tree_to_index(repo, &tree_obj, Path::new(""))?;
        }
        
        Ok(())
    }
    
    // Helper pentru a adăuga intrările unui arbore în index
    fn add_tree_to_index(repo: &mut Repository, tree_obj: &Box<dyn crate::core::database::database::GitObject>, path: &Path) -> Result<(), Error> {
        // Verifică dacă este un arbore
        if tree_obj.get_type() != "tree" {
            return Ok(());
        }
        
        // Convertim obiectul la Tree
        let tree = if let Some(t) = tree_obj.as_any().downcast_ref::<crate::core::database::tree::Tree>() {
            t
        } else {
            return Ok(());
        };
        
        // Adaugă fiecare intrare din arbore în index
        for (name, entry) in tree.get_entries() {
            let entry_path = path.join(name);
            
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    // Este un fișier, îl adăugăm direct în index
                    // Pentru a adăuga în index, avem nevoie de stat
                    if let Ok(stat) = std::fs::metadata(&repo.workspace.root_path.join(&entry_path)) {
                        repo.index.add(&entry_path, &oid, &stat)?;
                    } else {
                        // Dacă fișierul nu există în workspace, îl adăugăm fără stat
                        // Folosim o valoare dummy pentru stat (nu este ideal)
                        let empty_stat = std::fs::metadata(&repo.workspace.root_path).unwrap_or_else(|_| {
                            // Fallback în caz că metadata pentru root_path eșuează
                            std::fs::metadata("/").unwrap()
                        });
                        repo.index.add(&entry_path, &oid, &empty_stat)?;
                    }
                },
                TreeEntry::Tree(subtree) => {
                    // Este un director, încarcă-l recursiv
                    if let Some(subtree_oid) = subtree.get_oid() {
                        let subtree_obj = repo.database.load(subtree_oid)?;
                        Self::add_tree_to_index(repo, &subtree_obj, &entry_path)?;
                    }
                }
            }
        }
        
        Ok(())
    }
    
    // Resetează o cale specifică la starea din commit
    fn reset_path(repo: &mut Repository, commit_oid: &str, pathname: &Path) -> Result<(), Error> {
        Self::reset_tree(repo, commit_oid, Some(pathname))
    }
    
    // Hard reset - resetează HEAD, index și workspace la starea commit-ului specificat
    fn hard_reset(repo: &mut Repository, commit_oid: &str, force: bool) -> Result<(), Error> {
        // Calculăm diferențele între HEAD și commit-ul țintă
        let current_oid = repo.refs.read_head()?;
        let tree_diff = repo.tree_diff(current_oid.as_deref(), Some(commit_oid))?;
        
        // Creăm migrarea
        let mut migration = repo.migration(tree_diff);
        
        if force {
            // Dacă avem force, ștergem toate conflictele potențiale înainte de aplicare
            migration.remove_all_conflicts();
            println!("Force flag aplicat - ignorând conflictele potențiale");
            println!("Notă: Fișierele modificate în workspace pot necesita actualizare manuală");
        }
        
        // Aplicăm schimbările
        migration.apply_changes()?;
        
        Ok(())
    }
    
    // Colectează lista de fișiere care trebuie actualizate și OID-urile lor
    fn collect_files_to_update(repo: &Repository, tree_diff: &HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>) -> Result<Vec<(PathBuf, String)>, Error> {
        let mut updates = Vec::new();
        
        for (path, _) in tree_diff {
            let path_str = path.to_string_lossy().to_string();
            if let Some(entry) = repo.index.get_entry(&path_str) {
                updates.push((path.clone(), entry.get_oid().to_string()));
            }
        }
        
        Ok(updates)
    }
    
    // Utility pentru a afișa OID-ul prescurtat
    fn short_oid(oid: &str) -> String {
        if oid.len() >= 7 {
            oid[0..7].to_string()
        } else {
            oid.to_string()
        }
    }
} 