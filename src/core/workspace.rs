// src/core/workspace.rs
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use regex::Regex; // Asigură-te că ai adăugat `regex = "1"` în Cargo.toml
use crate::errors::error::Error;

pub struct Workspace {
    pub root_path: PathBuf,
}

impl Workspace {
    pub fn new(root_path: &Path) -> Self {
        Workspace {
            root_path: root_path.to_path_buf(),
        }
    }

    // Load ignore patterns from .ashignore
    fn load_ignore_patterns(&self) -> HashSet<String> {
        let mut patterns = HashSet::new();
        let ignore_path = self.root_path.join(".ashignore");

        // Always ignore .ash directory and .git directory
        patterns.insert(".ash".to_string());
        patterns.insert(".ash/".to_string()); // More explicit directory ignore
        patterns.insert(".git".to_string());
        patterns.insert(".git/".to_string());

        if ignore_path.exists() {
            if let Ok(content) = fs::read_to_string(ignore_path) {
                for line in content.lines() {
                    let line = line.trim();
                    if !line.is_empty() && !line.starts_with('#') {
                        patterns.insert(line.to_string());
                    }
                }
            }
        }

        patterns
    }

    // List files recursively, applying ignore patterns
    pub fn list_files(&self) -> Result<Vec<PathBuf>, Error> {
        let ignore_patterns = self.load_ignore_patterns();
        let mut files = Vec::new();
        self.list_files_recursive(&self.root_path, PathBuf::new(), &mut files, &ignore_patterns)?;
        Ok(files)
    }


     // Helper for recursive listing
     fn list_files_recursive(
         &self,
         abs_dir_path: &Path,
         rel_dir_path: PathBuf, // Pass relative path for checking ignores
         files: &mut Vec<PathBuf>,
         ignore_patterns: &HashSet<String>,
     ) -> Result<(), Error> {
         match fs::read_dir(abs_dir_path) {
             Ok(entries) => {
                 for entry_result in entries {
                     match entry_result {
                         Ok(entry) => {
                             let entry_abs_path = entry.path();
                             let file_name = entry.file_name();

                             // Construct relative path for ignore checking
                             let entry_rel_path = rel_dir_path.join(&file_name);
                             let rel_path_str = entry_rel_path.to_string_lossy().to_string().replace("\\", "/"); // Normalize

                             // --- Ignore Check ---
                             if self.matches_any_pattern(&rel_path_str, ignore_patterns) {
                                 // If the pattern specifically targets a directory (ends with /), ignore it and don't recurse
                                 // Also ignore if it's an exact match for a non-directory pattern
                                  if entry_abs_path.is_dir() {
                                       // Check if any pattern matches this directory specifically
                                       let dir_pattern_match = ignore_patterns.iter().any(|p| {
                                            let norm_p = p.replace("\\", "/");
                                            (norm_p.ends_with('/') && rel_path_str.starts_with(&norm_p[..norm_p.len()-1])) || norm_p == rel_path_str
                                       });
                                       if dir_pattern_match {
                                            //println!("Ignoring directory and contents: {}", rel_path_str);
                                            continue; // Skip recursion
                                       }
                                  } else {
                                       // If it's a file and matches any pattern, ignore it
                                       //println!("Ignoring file: {}", rel_path_str);
                                       continue;
                                  }
                             }
                             // --- End Ignore Check ---

                             if entry_abs_path.is_dir() {
                                 // Recursively scan subdirectories
                                 self.list_files_recursive(&entry_abs_path, entry_rel_path, files, ignore_patterns)?;
                             } else if entry_abs_path.is_file() {
                                 // Add file if it's not ignored
                                 files.push(entry_rel_path);
                             }
                         },
                         Err(e) => {
                              if e.kind() == std::io::ErrorKind::PermissionDenied {
                                   eprintln!("Warning: Permission denied reading entry in {}", abs_dir_path.display());
                                   continue;
                              } else {
                                   return Err(Error::IO(e));
                              }
                         }
                     }
                 }
                 Ok(())
             },
             Err(e) => {
                 if e.kind() == std::io::ErrorKind::PermissionDenied {
                     eprintln!("Warning: Permission denied reading directory {}", abs_dir_path.display());
                     Ok(())
                 } else {
                     Err(Error::IO(e))
                 }
             }
         }
     }

    // List files starting from a specific path (for add command)
    pub fn list_files_from(&self, start_path: &Path, index_entries: &HashMap<String, String>) -> Result<(Vec<PathBuf>, Vec<String>), Error> {
        let mut files_found = Vec::new();
        let mut files_missing = Vec::new();

        let abs_start_path = if start_path.is_absolute() {
            start_path.to_path_buf()
        } else {
            self.root_path.join(start_path)
        };

        if !abs_start_path.exists() {
             let rel_start_str = start_path.to_string_lossy();
             let prefix_to_check = format!("{}/", rel_start_str);
             let mut deleted_from_index = Vec::new();
             let mut found_match = false;
             for key in index_entries.keys() {
                 if key == &*rel_start_str || key.starts_with(&prefix_to_check) {
                     deleted_from_index.push(key.clone());
                     found_match = true;
                 }
             }
             if found_match {
                 return Ok((Vec::new(), deleted_from_index));
             } else {
                 return Err(Error::InvalidPath(format!(
                     "Pathspec '{}' did not match any files", start_path.display()
                 )));
             }
        }

        let rel_start_path = if abs_start_path == self.root_path {
            PathBuf::new()
        } else {
            match abs_start_path.strip_prefix(&self.root_path) {
                Ok(rel) => rel.to_path_buf(),
                Err(_) => return Err(Error::InvalidPath(format!(
                    "Cannot make '{}' relative to repository root", abs_start_path.display()
                )))
            }
        };

        let path_prefix = rel_start_path.to_string_lossy().to_string();
        let mut expected_files = HashSet::new();
        for index_path in index_entries.keys() {
             if index_path == &path_prefix || (path_prefix.is_empty()) || (index_path.starts_with(&format!("{}/", path_prefix))) {
                 expected_files.insert(index_path.clone());
             }
        }

        if abs_start_path.is_dir() {
            let ignore_patterns = self.load_ignore_patterns();
            self.process_directory( &abs_start_path, &rel_start_path, &ignore_patterns, &mut files_found, &mut expected_files )?;
             for missing_path in expected_files {
                  if missing_path == path_prefix || missing_path.starts_with(&format!("{}/", path_prefix)) || path_prefix.is_empty() {
                     files_missing.push(missing_path);
                  }
             }
        } else {
            let rel_path_str = rel_start_path.to_string_lossy().to_string();
            let ignore_patterns = self.load_ignore_patterns();
            if !self.matches_any_pattern(&rel_path_str, &ignore_patterns) {
                files_found.push(rel_start_path);
            }
            expected_files.remove(&rel_path_str);
             files_missing.extend(expected_files.into_iter());
        }
        Ok((files_found, files_missing))
    }

    // Helper for processing directories recursively
    fn process_directory(
        &self,
        abs_path: &Path,
        rel_path: &Path,
        ignore_patterns: &HashSet<String>,
        files: &mut Vec<PathBuf>,
        expected_files: &mut HashSet<String>
    ) -> Result<(), Error> {
        match fs::read_dir(abs_path) {
            Ok(entries) => {
                for entry_result in entries {
                    match entry_result {
                        Ok(entry) => {
                            let entry_path = entry.path();
                            let file_name = entry.file_name();
                            let entry_rel_path = rel_path.join(&file_name);
                            let rel_path_str = entry_rel_path.to_string_lossy().to_string().replace("\\", "/");

                            if self.matches_any_pattern(&rel_path_str, ignore_patterns) {
                                 // Skip ignored paths entirely
                                continue;
                            }

                            if entry_path.is_dir() {
                                self.process_directory( &entry_path, &entry_rel_path, ignore_patterns, files, expected_files )?;
                            } else if entry_path.is_file() {
                                files.push(entry_rel_path.clone());
                                expected_files.remove(&rel_path_str);
                            }
                        },
                        Err(e) => {
                            if e.kind() == std::io::ErrorKind::PermissionDenied {
                                 eprintln!("Warning: Permission denied reading entry in {}", abs_path.display());
                                 continue;
                            } else { return Err(Error::IO(e)); }
                        }
                    }
                }
                Ok(())
            },
            Err(e) => {
                 if e.kind() == std::io::ErrorKind::PermissionDenied {
                      eprintln!("Warning: Permission denied accessing directory {}", abs_path.display());
                      Ok(())
                 } else { Err(Error::IO(e)) }
            }
        }
    }


    // Check if a path matches any ignore pattern
    fn matches_any_pattern(&self, path_str: &str, patterns: &HashSet<String>) -> bool {
         let normalized_path = path_str.replace("\\", "/");
         let path_to_match = if Path::new(&normalized_path).is_absolute() {
              match Path::new(&normalized_path).strip_prefix(&self.root_path) {
                   Ok(p) => p.to_string_lossy().to_string(),
                   Err(_) => normalized_path,
              }
         } else { normalized_path };

        for pattern in patterns {
             let normalized_pattern = pattern.replace("\\", "/");
            if self.matches_pattern(&path_to_match, &normalized_pattern) {
                return true;
            }
        }
        false
    }

    // Simple pattern matching logic
    fn matches_pattern(&self, path: &str, pattern: &str) -> bool {
        if pattern.is_empty() { return false; }

        // Handle directory patterns (ending with /)
        if pattern.ends_with('/') {
            let dir_pattern = &pattern[0..pattern.len() - 1];
            // Match directory itself or anything inside it
            return path == dir_pattern || path.starts_with(&format!("{}/", dir_pattern));
        }

        // Handle file patterns (no slashes or specific file match)
        if !pattern.contains('/') {
            // Match filename anywhere in the path
            if let Some(file_name) = Path::new(path).file_name() {
                if file_name == std::ffi::OsStr::new(pattern) {
                    return true;
                }
            }
             // Basic wildcard matching for filename
             if pattern.contains('*') {
                  if let Some(filename) = Path::new(path).file_name().and_then(|s| s.to_str()) {
                       let regex_pattern_str = pattern.replace(".", "\\.").replace("*", ".*");
                       let filename_regex = format!("^{}$", regex_pattern_str);
                       if let Ok(re_fn) = Regex::new(&filename_regex) {
                           if re_fn.is_match(filename) { return true; }
                       }
                  }
             }

        } else { // Pattern contains slashes (specific path)
            // Basic wildcard matching for path
            if pattern.contains('*') {
                let regex_pattern_str = pattern.replace(".", "\\.").replace("*", ".*");
                 // Anchor the pattern to the beginning for path match
                let final_regex_str = format!("^{}", regex_pattern_str);
                 if let Ok(re) = Regex::new(&final_regex_str) {
                     if re.is_match(path) { return true; }
                 }
            } else {
                 // Exact path match or prefix match if pattern represents a directory
                 if path == pattern || path.starts_with(&format!("{}/", pattern)) {
                    return true;
                 }
            }
        }


        false
    }


    pub fn read_file(&self, path: &Path) -> Result<Vec<u8>, Error> {
        let file_path = self.root_path.join(path);
        match fs::read(&file_path) {
            Ok(data) => Ok(data),
            Err(e) => Err(Error::IO(e)), // Simplify error handling for now
        }
    }

    pub fn stat_file(&self, path: &Path) -> Result<fs::Metadata, Error> {
        let file_path = self.root_path.join(path);
        match fs::metadata(&file_path) {
            Ok(metadata) => Ok(metadata),
            Err(e) => Err(Error::IO(e)), // Simplify error handling
        }
    }

    pub fn path_exists(&self, path: &Path) -> Result<bool, Error> {
        let file_path = self.root_path.join(path);
        Ok(file_path.exists())
    }

    pub fn write_file(&self, path: &Path, data: &[u8]) -> Result<(), Error> {
        let full_path = self.root_path.join(path);
        if let Some(parent) = full_path.parent() {
             if !parent.exists() {
                //println!("Creating parent directory for write: {}", parent.display());
                std::fs::create_dir_all(parent).map_err(Error::IO)?;
             }
        }
        //println!("Writing file: {} ({} bytes)", full_path.display(), data.len());
        std::fs::write(&full_path, data).map_err(Error::IO)
    }

    // Includes logging added previously
    pub fn remove_file(&self, path: &Path) -> Result<(), Error> {
        let full_path = self.root_path.join(path);
        println!("  Attempting to remove file/dir at: {}", full_path.display());
        if full_path.exists() {
            if full_path.is_file() {
                println!("    Path is a file, calling std::fs::remove_file");
                match std::fs::remove_file(&full_path) {
                    Ok(_) => println!("    std::fs::remove_file succeeded for file."),
                    Err(e) => {
                        println!("    Error removing file: {}", e);
                        return Err(Error::IO(e));
                    }
                }
            } else if full_path.is_dir() {
                 println!("    Warning: remove_file called on a directory: {}. Use force_remove_directory instead.", full_path.display());
                 return Err(Error::Generic(format!("Attempted to use remove_file on directory: {}", full_path.display())));
            } else {
                println!("    Path exists but is not a file or directory (e.g., symlink?): {}", full_path.display());
                  match std::fs::remove_file(&full_path) { // Try removing anyway
                     Ok(_) => println!("    Successfully removed non-file/non-dir path."),
                     Err(e) => {
                          println!("    Error removing non-file/non-dir path: {}", e);
                          return Err(Error::IO(e));
                     }
                  }
            }
        } else {
             println!("    Path does not exist, nothing to remove: {}", full_path.display());
        }
        if self.root_path.join(path).exists() { // Re-check using relative path construction logic
             println!("    Warning: Path still exists after removal attempt: {}", full_path.display());
        } else {
             println!("    Path confirmed removed or did not exist initially: {}", full_path.display());
        }
        Ok(())
    }

    pub fn remove_directory(&self, path: &Path) -> Result<(), Error> {
        let full_path = self.root_path.join(path);
        if !full_path.exists() || !full_path.is_dir() { return Ok(()); }
        let is_effectively_empty = match std::fs::read_dir(&full_path) {
            Ok(entries) => !entries.filter_map(Result::ok).any(|e| !e.file_name().to_string_lossy().starts_with('.')),
            Err(_) => false,
        };
        if is_effectively_empty {
            println!("Attempting to remove empty/effectively empty directory: {}", full_path.display());
            if let Err(e) = std::fs::remove_dir(&full_path) {
                eprintln!("Warning: Failed to remove directory {} with std::fs::remove_dir: {}", full_path.display(), e);
            } else {
                 println!("Successfully removed empty directory: {}", full_path.display());
            }
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && parent.to_string_lossy() != "." {
                     let _ = self.remove_directory(parent); // Best effort cleanup
                }
            }
        } else {
            println!("Directory not empty, skipping removal: {}", full_path.display());
        }
        Ok(())
    }

    pub fn make_directory(&self, path: &Path) -> Result<(), Error> {
        let full_path = self.root_path.join(path);
        if full_path.exists() {
            if full_path.is_file() {
                println!("Path {} exists as file, removing to create directory.", full_path.display());
                std::fs::remove_file(&full_path).map_err(Error::IO)?;
                 println!("Creating directory: {}", full_path.display());
                 std::fs::create_dir_all(&full_path).map_err(Error::IO)
            } else { Ok(()) }
        } else {
            //println!("Creating directory: {}", full_path.display()); // Reduce noise
            std::fs::create_dir_all(&full_path).map_err(Error::IO)
        }
    }

    pub fn force_remove_directory(&self, path: &Path) -> Result<(), Error> {
        let full_path = self.root_path.join(path);
        if full_path.exists() && full_path.is_dir() {
            println!("Force removing directory and contents: {}", full_path.display());
             match std::fs::remove_dir_all(&full_path) {
                 Ok(_) => {
                     println!("  Successfully force removed directory: {}", full_path.display());
                     Ok(())
                 },
                 Err(e) => {
                      println!("  Warning: Failed to force remove directory {}: {}", full_path.display(), e);
                     Err(Error::IO(e))
                 }
             }
        } else if full_path.exists() {
             println!("Warning: force_remove_directory called on non-directory path: {}", full_path.display());
             self.remove_file(path) // Attempt to remove as file
        } else {
             //println!("Directory does not exist, nothing to force remove: {}", full_path.display()); // Reduce noise
             Ok(())
        }
    }
}