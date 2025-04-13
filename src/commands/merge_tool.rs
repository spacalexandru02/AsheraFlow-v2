// src/commands/merge_tool.rs
use std::process::Command;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::env;
use std::io::{self, Write};
use std::collections::{HashMap, HashSet};

use crate::errors::error::Error;
use crate::core::index::index::Index;
use crate::core::workspace::Workspace;
use crate::core::database::database::Database;
use crate::core::database::blob::Blob;
use crate::core::refs::Refs;
use crate::core::color::Color;
use crate::core::file_mode::FileMode;
use crate::core::diff::diff;

pub struct MergeToolCommand;

// Constants for conflict markers
const MERGE_MARKER_OURS_BEGIN: &str = "<<<<<<< OURS\n";
const MERGE_MARKER_MIDDLE: &str = "=======\n";
const MERGE_MARKER_THEIRS_END: &str = ">>>>>>> THEIRS\n";
const MERGE_MARKER_BASE_BEGIN: &str = "||||||| BASE\n";

// Structure to hold conflict information
struct ConflictInfo {
    path_str: String,
    path: PathBuf,
    base_oid: Option<String>,   // stage 1
    ours_oid: Option<String>,   // stage 2
    theirs_oid: Option<String>, // stage 3
}

impl MergeToolCommand {
    pub fn execute(tool: Option<&str>) -> Result<(), Error> {
        let start_time = Instant::now();
        
        println!("Starting merge resolution tool...");
        
        // Initialize repository components
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository (or any of the parent directories): .ash directory not found".into()));
        }
        
        let workspace = Workspace::new(root_path);
        let mut database = Database::new(git_path.join("objects"));
        let mut index = Index::new(git_path.join("index"));
        
        // Try to acquire the lock on the index
        if !index.load_for_update()? {
            return Err(Error::Lock(format!(
                "Unable to acquire lock on index. Another process may be using it. \
                If not, the .ash/index.lock file may need to be manually removed."
            )));
        }
        
        // Check if there are conflicts to resolve
        if !index.has_conflict() {
            println!("{}", Color::green("No merge conflicts found."));
            index.rollback()?;
            return Ok(());
        }
        
        // Get conflicted paths
        let conflicted_paths = index.conflict_paths();
        println!("Found {} conflicted {}.", 
            Color::red(&conflicted_paths.len().to_string()),
            if conflicted_paths.len() == 1 { "file" } else { "files" }
        );
        
        // Find available editors
        let editor = Self::get_editor(tool)?;
        println!("Using editor: {}", Color::cyan(&editor));
        
        // Keep track of resolved and skipped files
        let mut resolved_count = 0;
        let mut skipped_count = 0;
        
        // Build a map of all conflict entries by path
        let mut conflict_entries: HashMap<String, Vec<(String, u8)>> = HashMap::new();
        
        // Collect all conflict entries from the index
        for entry in index.each_entry() {
            if entry.stage > 0 {
                let path_str = entry.get_path().to_string();
                let entry_info = (entry.get_oid().to_string(), entry.stage);
                
                println!("Found conflict entry: {} (stage {})", path_str, entry.stage);
                
                // Add to our conflict map
                if !conflict_entries.contains_key(&path_str) {
                    conflict_entries.insert(path_str.clone(), Vec::new());
                }
                conflict_entries.get_mut(&path_str).unwrap().push(entry_info);
            }
        }
        
        // Process each conflicted path
        for path_str in &conflicted_paths {
            let path = PathBuf::from(path_str);
            
            // Check if this is a directory
            let full_path = workspace.root_path.join(&path);
            if full_path.exists() && full_path.is_dir() {
                println!("\nDirectory conflict detected: {}", Color::yellow(path_str));
                println!("Exploring directory for conflicted files...");
                
                // Explore the directory for conflict files
                let (resolved, skipped) = Self::explore_directory_for_conflicts(
                    &workspace, &mut database, &mut index, &path, &conflict_entries, &editor
                )?;
                
                resolved_count += resolved;
                skipped_count += skipped;
                continue;
            }
            
            // Process regular file conflict
            if let Some(entries) = conflict_entries.get(path_str) {
                let mut info = ConflictInfo {
                    path_str: path_str.clone(),
                    path: path.clone(),
                    base_oid: None,
                    ours_oid: None,
                    theirs_oid: None,
                };
                
                // Extract stage information
                for (oid, stage) in entries {
                    match stage {
                        1 => info.base_oid = Some(oid.clone()),
                        2 => info.ours_oid = Some(oid.clone()),
                        3 => info.theirs_oid = Some(oid.clone()),
                        _ => {}
                    }
                }
                
                // Process this conflict
                match Self::process_conflict(&workspace, &mut database, &mut index, &info, &editor) {
                    Ok(true) => resolved_count += 1,
                    Ok(false) => skipped_count += 1,
                    Err(e) => {
                        println!("Error processing conflict: {}", e);
                        skipped_count += 1;
                    }
                }
            }
        }
        
        // Save index with potentially resolved conflicts
        if index.is_changed() {
            index.write_updates()?;
            println!("\nUpdated index written successfully.");
        } else {
            index.rollback()?;
            println!("\nNo changes made to index.");
        }
        
        // Check if all conflicts were resolved
        if !index.has_conflict() {
            println!("{} All conflicts resolved. You can now commit the results.", Color::green("✓"));
        } else {
            println!("{} There are still unresolved conflicts.", Color::yellow("!"));
            println!("Use 'ash merge --tool' to continue resolving conflicts.");
        }
        
        // Print summary
        let elapsed = start_time.elapsed();
        println!("\nMerge tool completed in {:.2}s", elapsed.as_secs_f32());
        println!("Files resolved: {}", Color::green(&resolved_count.to_string()));
        println!("Files skipped: {}", Color::yellow(&skipped_count.to_string()));
        println!("Conflicts remaining: {}", Color::red(&index.conflict_paths().len().to_string()));
        
        Ok(())
    }
    
    // New method to explore directory for conflicts
    fn explore_directory_for_conflicts(
        workspace: &Workspace,
        database: &mut Database,
        index: &mut Index,
        dir_path: &Path,
        conflict_entries: &HashMap<String, Vec<(String, u8)>>,
        editor: &str
    ) -> Result<(usize, usize), Error> {
        let mut resolved_count = 0;
        let mut skipped_count = 0;
        
        // Get file conflicts under this directory prefix
        let dir_path_str = dir_path.to_string_lossy().to_string();
        let dir_prefix = if dir_path_str.ends_with('/') {
            dir_path_str.clone()
        } else {
            format!("{}/", dir_path_str)
        };
        
        println!("DEBUG: Exploring directory: {}", dir_path.display());
        
        // Find all conflict entries under this directory
        let mut files_to_process = Vec::new();
        
        // Check for exact directory conflict match
        let dir_is_conflict = conflict_entries.contains_key(&dir_path_str);
        if dir_is_conflict {
            println!("DEBUG: Directory itself is marked as a conflict: {}", dir_path_str);
        }
        
        // Recursively explore the physical directory structure to find potential conflict files
        if workspace.root_path.join(dir_path).exists() {
            Self::explore_physical_directory(
                workspace, 
                dir_path, 
                &mut files_to_process, 
                conflict_entries
            )?;
        }
        
        // Also collect conflicts from the index that match our directory prefix
        for (conflict_path, entries) in conflict_entries {
            // If the conflict path is within this directory
            if conflict_path == &dir_path_str || conflict_path.starts_with(&dir_prefix) {
                println!("DEBUG: Found conflict path under directory: {}", conflict_path);
                
                // Check if we've already added this path
                if !files_to_process.iter().any(|info| info.path_str == *conflict_path) {
                    // Extract stage info to create ConflictInfo
                    let mut info = ConflictInfo {
                        path_str: conflict_path.clone(),
                        path: PathBuf::from(conflict_path),
                        base_oid: None,
                        ours_oid: None,
                        theirs_oid: None,
                    };
                    
                    for (oid, stage) in entries {
                        match stage {
                            1 => info.base_oid = Some(oid.clone()),
                            2 => info.ours_oid = Some(oid.clone()),
                            3 => info.theirs_oid = Some(oid.clone()),
                            _ => {}
                        }
                    }
                    
                    files_to_process.push(info);
                }
            }
        }
        
        // Check if we have any files to process
        let files_count = files_to_process.len();
        
        // Process each conflicted file found
        for info in &files_to_process {
            println!("Processing conflict file: {}", Color::yellow(&info.path_str));
            
            // Process this conflict
            match Self::process_conflict(workspace, database, index, info, editor) {
                Ok(true) => resolved_count += 1,
                Ok(false) => skipped_count += 1,
                Err(e) => {
                    println!("Error processing conflict: {}", e);
                    skipped_count += 1;
                }
            }
        }
        
        // If we didn't find any conflict files and directory itself is a conflict,
        // add it to skipped count
        if files_count == 0 {
            if dir_is_conflict {
                println!("Directory {} is a conflict but no conflict files found.", dir_path_str);
                
                // Try to explore subdirectories for conflicts
                let full_dir_path = workspace.root_path.join(dir_path);
                if full_dir_path.exists() && full_dir_path.is_dir() {
                    if let Ok(entries) = fs::read_dir(&full_dir_path) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            let rel_path = path.strip_prefix(&workspace.root_path)
                                .unwrap_or(&path);
                            
                            if path.is_dir() {
                                println!("Recursively exploring subdirectory: {}", rel_path.display());
                                let (sub_resolved, sub_skipped) = Self::explore_directory_for_conflicts(
                                    workspace, database, index, rel_path, conflict_entries, editor
                                )?;
                                
                                resolved_count += sub_resolved;
                                skipped_count += sub_skipped;
                            }
                        }
                    }
                }
                
                if resolved_count == 0 {
                    skipped_count += 1;
                }
            } else {
                println!("No conflict files found in directory: {}", dir_path.display());
            }
        }
        
        Ok((resolved_count, skipped_count))
    }
    
    // Helper method to recursively explore physical directories for conflicts
    fn explore_physical_directory(
        workspace: &Workspace,
        dir_path: &Path,
        files_to_process: &mut Vec<ConflictInfo>,
        conflict_entries: &HashMap<String, Vec<(String, u8)>>
    ) -> Result<(), Error> {
        let full_dir_path = workspace.root_path.join(dir_path);
        
        if !full_dir_path.exists() || !full_dir_path.is_dir() {
            return Ok(());
        }
        
        // Read the directory entries
        let entries = fs::read_dir(&full_dir_path)?;
        
        for entry in entries.flatten() {
            let path = entry.path();
            let rel_path = path.strip_prefix(&workspace.root_path)
                .unwrap_or(&path)
                .to_path_buf();
            
            let path_str = rel_path.to_string_lossy().to_string();
            println!("DEBUG: Found entry: {}", path.display());
            println!("DEBUG: Relative path: {}", path_str);
            
            if path.is_file() {
                println!("DEBUG: Found file: {}", path_str);
                
                // Check if this file has conflict entries
                if let Some(entries) = conflict_entries.get(&path_str) {
                    println!("DEBUG: Found conflict entries for file: {}", path_str);
                    
                    let mut info = ConflictInfo {
                        path_str: path_str.clone(),
                        path: rel_path.clone(),
                        base_oid: None,
                        ours_oid: None,
                        theirs_oid: None,
                    };
                    
                    for (oid, stage) in entries {
                        match stage {
                            1 => info.base_oid = Some(oid.clone()),
                            2 => info.ours_oid = Some(oid.clone()),
                            3 => info.theirs_oid = Some(oid.clone()),
                            _ => {}
                        }
                    }
                    
                    files_to_process.push(info);
                } else {
                    println!("DEBUG: No conflict entries found for file: {}", path_str);
                }
            } else if path.is_dir() {
                println!("DEBUG: Found directory: {}", path_str);
                
                // Recursively explore this directory
                Self::explore_physical_directory(
                    workspace, 
                    &rel_path, 
                    files_to_process, 
                    conflict_entries
                )?;
            }
        }
        
        Ok(())
    }
    
    // Process a single conflict file
    fn process_conflict(
        workspace: &Workspace,
        database: &mut Database,
        index: &mut Index,
        info: &ConflictInfo,
        editor: &str
    ) -> Result<bool, Error> {
        let path_str = &info.path_str;
        let path = &info.path;
        
        println!("Processing conflict in file: {}", Color::yellow(path_str));
        
        // Debug conflict info
        println!("  Base OID: {:?}", info.base_oid);
        println!("  Ours OID: {:?}", info.ours_oid);
        println!("  Theirs OID: {:?}", info.theirs_oid);
        
        // Check if this is a directory
        let full_path = workspace.root_path.join(path);
        if full_path.exists() && full_path.is_dir() {
            println!("  This is a directory conflict. Checking for actual conflicting files...");
            
            // Try to find actual conflict files within the directory
            let dir_conflicts = Self::find_directory_conflict_files(
                workspace, database, path, 
                info.base_oid.as_deref(), 
                info.ours_oid.as_deref(), 
                info.theirs_oid.as_deref()
            )?;
            
            if dir_conflicts.is_empty() {
                println!("  No specific file conflicts found in directory");
                return Ok(false);
            }
            
            // Process each conflicting file
            let mut all_resolved = true;
            for (rel_path, file_info) in dir_conflicts {
                println!("  Processing specific file conflict: {}", rel_path.display());
                match Self::process_conflict(workspace, database, index, &file_info, editor) {
                    Ok(true) => println!("    ✓ Resolved conflict in file: {}", rel_path.display()),
                    Ok(false) => {
                        println!("    ✗ Failed to resolve conflict in file: {}", rel_path.display());
                        all_resolved = false;
                    },
                    Err(e) => {
                        println!("    ✗ Error processing file conflict: {}", e);
                        all_resolved = false;
                    }
                }
            }
            
            // If all inner conflicts were resolved, mark the directory conflict as resolved
            if all_resolved {
                // Remove directory conflict
                index.resolve_directory_conflict(path)?;
                println!("  ✓ All directory conflicts resolved");
                return Ok(true);
            }
            
            return Ok(false);
        }
        
        // Create conflict-marked file for regular file conflicts
        if let Err(e) = Self::create_conflict_file(workspace, database, path, 
                                 info.base_oid.as_deref(), 
                                 info.ours_oid.as_deref(), 
                                 info.theirs_oid.as_deref()) {
            println!("  {} Error creating conflict file: {}", Color::red("✗"), e);
            return Ok(false);
        }
        
        // Offer options for resolution
        println!("Options for conflict in {}:", Color::yellow(path_str));
        println!("  1. Open in editor ({}) to resolve manually", editor);
        println!("  2. Accept 'ours' version");
        println!("  3. Accept 'theirs' version");
        println!("  4. Skip this file");
        println!("  q. Quit resolution process");
        
        let mut choice = String::new();
        print!("Enter choice [1]: ");
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut choice).unwrap();
        let choice = choice.trim();
        
        match choice {
            "" | "1" => {
                // Use editor to resolve conflicts
                if let Err(e) = Self::open_editor(path, editor) {
                    println!("  {} Error opening editor: {}", Color::red("✗"), e);
                    return Ok(false);
                }
                
                // Check if conflict was resolved
                match Self::is_conflict_resolved(path, workspace) {
                    Ok(true) => {
                        // Update index with resolved file
                        let stat = workspace.stat_file(path)?;
                        let file_contents = workspace.read_file(path)?;
                        let mut blob = Blob::new(file_contents);
                        database.store(&mut blob)?;
                        let oid = blob.get_oid().unwrap().clone();
                        
                        // Resolve conflict in index
                        index.resolve_conflict(path, &oid, &stat)?;
                        println!("  {} Conflict resolved for file: {}", Color::green("✓"), path_str);
                        return Ok(true);
                    },
                    Ok(false) => {
                        println!("  {} Conflict markers still present, conflict not resolved.", Color::red("✗"));
                        return Ok(false);
                    },
                    Err(e) => {
                        println!("  {} Error checking if conflict was resolved: {}", Color::red("✗"), e);
                        return Ok(false);
                    }
                }
            },
            "2" => {
                // Accept "ours" version
                if let Some(oid) = &info.ours_oid {
                    let obj = match database.load(oid) {
                        Ok(obj) => obj,
                        Err(e) => {
                            println!("  {} Error loading 'ours' version: {}", Color::red("✗"), e);
                            return Ok(false);
                        }
                    };
                    
                    let content = obj.to_bytes();
                    if let Err(e) = workspace.write_file(path, &content) {
                        println!("  {} Error writing 'ours' version: {}", Color::red("✗"), e);
                        return Ok(false);
                    }
                    
                    let stat = match workspace.stat_file(path) {
                        Ok(stat) => stat,
                        Err(e) => {
                            println!("  {} Error getting file stats: {}", Color::red("✗"), e);
                            return Ok(false);
                        }
                    };
                    
                    // Resolve the conflict
                    if let Err(e) = index.resolve_conflict(path, oid, &stat) {
                        println!("  {} Error resolving conflict in index: {}", Color::red("✗"), e);
                        return Ok(false);
                    }
                    
                    println!("  {} Accepted 'ours' version for file: {}", Color::green("✓"), path_str);
                    return Ok(true);
                } else {
                    // Check if file exists in workspace
                    let full_path = workspace.root_path.join(path);
                    if full_path.exists() && !full_path.is_dir() {
                        // Read the file content
                        match std::fs::read(&full_path) {
                            Ok(content) => {
                                // Check if the file has conflict markers
                                let content_str = String::from_utf8_lossy(&content).to_string();
                                
                                // If the file has conflict markers, try to extract just the "ours" part
                                if content_str.contains("<<<<<<< OURS") || content_str.contains("<<<<<<<") {
                                    println!("    Extracting 'ours' content from conflict markers");
                                    
                                    // Extract the content between the "ours" markers
                                    let mut ours_content = String::new();
                                    
                                    // Find the start and end of the "ours" section
                                    let ours_start_marker = if content_str.contains("<<<<<<< OURS") {
                                        "<<<<<<< OURS"
                                    } else {
                                        "<<<<<<<"
                                    };
                                    
                                    let parts: Vec<&str> = content_str.split(ours_start_marker).collect();
                                    if parts.len() > 1 {
                                        // Take everything after the marker until "======="
                                        let ours_section = parts[1].split("=======").next().unwrap_or("");
                                        ours_content = ours_section.trim().to_string();
                                        
                                        println!("    Extracted content ({} bytes):\n{}", ours_content.len(), ours_content);
                                        
                                        // Use the extracted "ours" content
                                        let ours_content_bytes = ours_content.clone().into_bytes();
                                        let mut blob = Blob::new(ours_content_bytes);
                                        if let Err(e) = database.store(&mut blob) {
                                            println!("  {} Error storing extracted content: {}", Color::red("✗"), e);
                                            return Ok(false);
                                        }
                                        
                                        if let Some(oid) = blob.get_oid() {
                                            // Write the clean content back to the file
                                            if let Err(e) = workspace.write_file(path, ours_content.as_bytes()) {
                                                println!("  {} Error writing extracted content: {}", Color::red("✗"), e);
                                                return Ok(false);
                                            }
                                            
                                            let stat = match workspace.stat_file(path) {
                                                Ok(stat) => stat,
                                                Err(e) => {
                                                    println!("  {} Error getting file stats: {}", Color::red("✗"), e);
                                                    return Ok(false);
                                                }
                                            };
                                            
                                            if let Err(e) = index.resolve_conflict(path, oid, &stat) {
                                                println!("  {} Error resolving conflict in index: {}", Color::red("✗"), e);
                                                return Ok(false);
                                            }
                                            
                                            println!("  {} Extracted 'ours' version from conflict markers: {}", 
                                                Color::green("✓"), path_str);
                                            return Ok(true);
                                        } else {
                                            println!("  {} Failed to get OID for blob", Color::red("✗"));
                                            return Ok(false);
                                        }
                                    } else {
                                        println!("  {} Could not parse conflict markers correctly", Color::red("✗"));
                                        return Ok(false);
                                    }
                                } else {
                                    // No conflict markers found, use the file as is (but this is unlikely)
                                    let mut blob = Blob::new(content.to_vec());
                                    if let Err(e) = database.store(&mut blob) {
                                        println!("  {} Error storing workspace file: {}", Color::red("✗"), e);
                                        return Ok(false);
                                    }
                                    
                                    if let Some(oid) = blob.get_oid() {
                                        let stat = match workspace.stat_file(path) {
                                            Ok(stat) => stat,
                                            Err(e) => {
                                                println!("  {} Error getting file stats: {}", Color::red("✗"), e);
                                                return Ok(false);
                                            }
                                        };
                                        
                                        if let Err(e) = index.resolve_conflict(path, oid, &stat) {
                                            println!("  {} Error resolving conflict in index: {}", Color::red("✗"), e);
                                            return Ok(false);
                                        }
                                        
                                        println!("  {} Used workspace file as 'ours' version: {}", 
                                             Color::green("✓"), path_str);
                                        return Ok(true);
                                    } else {
                                        println!("  {} Failed to get OID for blob", Color::red("✗"));
                                        return Ok(false);
                                    }
                                }
                            },
                            Err(e) => {
                                println!("  {} Error reading workspace file: {}", Color::red("✗"), e);
                                return Ok(false);
                            }
                        }
                    } else {
                        println!("  {} No 'ours' version available.", Color::red("✗"));
                        return Ok(false);
                    }
                }
            },
            "3" => {
                // Accept "theirs" version
                if let Some(oid) = &info.theirs_oid {
                    let obj = match database.load(oid) {
                        Ok(obj) => obj,
                        Err(e) => {
                            println!("  {} Error loading 'theirs' version: {}", Color::red("✗"), e);
                            return Ok(false);
                        }
                    };
                    
                    let content = obj.to_bytes();
                    if let Err(e) = workspace.write_file(path, &content) {
                        println!("  {} Error writing 'theirs' version: {}", Color::red("✗"), e);
                        return Ok(false);
                    }
                    
                    let stat = match workspace.stat_file(path) {
                        Ok(stat) => stat,
                        Err(e) => {
                            println!("  {} Error getting file stats: {}", Color::red("✗"), e);
                            return Ok(false);
                        }
                    };
                    
                    // Resolve the conflict
                    if let Err(e) = index.resolve_conflict(path, oid, &stat) {
                        println!("  {} Error resolving conflict in index: {}", Color::red("✗"), e);
                        return Ok(false);
                    }
                    
                    println!("  {} Accepted 'theirs' version for file: {}", Color::green("✓"), path_str);
                    return Ok(true);
                } else {
                    println!("  {} No 'theirs' version available.", Color::red("✗"));
                    return Ok(false);
                }
            },
            "4" => {
                println!("  Skipped file: {}", path_str);
                return Ok(false);
            },
            "q" | "Q" => {
                println!("Quitting resolution process.");
                return Err(Error::Generic("User quit resolution process".to_string()));
            },
            _ => {
                println!("Invalid choice. Skipping file.");
                return Ok(false);
            }
        }
    }
    
    // Find a usable editor
    fn get_editor(tool: Option<&str>) -> Result<String, Error> {
        // First, check if user explicitly specified a tool
        if let Some(tool_name) = tool {
            return Self::check_tool_available(tool_name);
        }
        
        // Next, check environment variables
        if let Ok(editor) = env::var("ASH_EDITOR") {
            return Self::check_tool_available(&editor);
        }
        
        if let Ok(editor) = env::var("EDITOR") {
            return Self::check_tool_available(&editor);
        }
        
        if let Ok(editor) = env::var("VISUAL") {
            return Self::check_tool_available(&editor);
        }
        
        // Try common editors in order of preference
        let common_editors = ["vim", "nano", "emacs", "vi"];
        for editor in common_editors {
            if Self::is_command_available(editor) {
                return Ok(editor.to_string());
            }
        }
        
        // If none found, default to vi which is most likely to be available
        Ok("vi".to_string())
    }
    
    // Check if a specific tool is available
    fn check_tool_available(tool: &str) -> Result<String, Error> {
        if Self::is_command_available(tool) {
            Ok(tool.to_string())
        } else {
            Err(Error::Generic(format!("Specified editor '{}' not found in PATH", tool)))
        }
    }
    
    // Check if a command is available in PATH
    fn is_command_available(cmd: &str) -> bool {
        // Extract the base command (without arguments)
        let cmd_parts: Vec<&str> = cmd.split_whitespace().collect();
        let base_cmd = cmd_parts[0];
        
        let check_cmd = if cfg!(target_os = "windows") {
            Command::new("where")
                .arg(base_cmd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        } else {
            Command::new("which")
                .arg(base_cmd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        };
        
        match check_cmd {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }
    
    // Create a file with conflict markers for editing
    fn create_conflict_file(
        workspace: &Workspace,
        database: &mut Database,
        path: &Path,
        base_oid: Option<&str>,
        ours_oid: Option<&str>,
        theirs_oid: Option<&str>
    ) -> Result<(), Error> {
        // Ensure file is not a directory
        let full_path = workspace.root_path.join(path);
        if full_path.exists() && full_path.is_dir() {
            return Err(Error::Generic(format!("Cannot create conflict markers for directory: {}", path.display())));
        }
        
        // Ensure parent directories exist
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                workspace.make_directory(parent)?;
            }
        }

        // Check if the file exists in the workspace but is not in index
        let ours_content_from_workspace = if full_path.exists() && !full_path.is_dir() {
            match std::fs::read(&full_path) {
                Ok(content) => Some(content),
                Err(_) => None,
            }
        } else {
            None
        };

        // Prepare content from all stages
        let base_content = if let Some(oid) = base_oid {
            let obj = database.load(oid)?;
            Some(obj.to_bytes())
        } else {
            None
        };

        let ours_content = if let Some(oid) = ours_oid {
            let obj = database.load(oid)?;
            Some(obj.to_bytes())
        } else {
            ours_content_from_workspace
        };

        let theirs_content = if let Some(oid) = theirs_oid {
            let obj = database.load(oid)?;
            Some(obj.to_bytes())
        } else {
            None
        };

        // Convert to strings or use empty strings if None
        let base_str = base_content.map_or(String::new(), |content| String::from_utf8_lossy(&content).to_string());
        let ours_str = ours_content.map_or(String::new(), |content| String::from_utf8_lossy(&content).to_string());
        let theirs_str = theirs_content.map_or(String::new(), |content| String::from_utf8_lossy(&content).to_string());

        // Check if there's a real conflict
        if ours_str == theirs_str {
            // No conflict - contents are identical, use either version
            if !ours_str.is_empty() {
                workspace.write_file(path, ours_str.as_bytes())?;
            } else if !theirs_str.is_empty() {
                workspace.write_file(path, theirs_str.as_bytes())?;
            }
            return Ok(());
        }

        // Determine the type of conflict
        let has_ours = !ours_str.is_empty();
        let has_theirs = !theirs_str.is_empty();
        let has_base = !base_str.is_empty();

        // Prepare conflict output with intelligent handling of diffs
        let mut conflict_content = String::new();

        if !has_ours && has_theirs {
            // File only exists in theirs
            conflict_content.push_str("<<<<<<< OURS (file doesn't exist)\n");
            conflict_content.push_str("=======\n");
            conflict_content.push_str(&theirs_str);
            conflict_content.push_str(">>>>>>> THEIRS\n");
        } else if has_ours && !has_theirs {
            // File only exists in ours
            conflict_content.push_str("<<<<<<< OURS\n");
            conflict_content.push_str(&ours_str);
            conflict_content.push_str("=======\n");
            conflict_content.push_str(">>>>>>> THEIRS (file doesn't exist)\n");
        } else {
            // Both versions exist, compare line by line
            let ours_lines: Vec<&str> = ours_str.lines().collect();
            let theirs_lines: Vec<&str> = theirs_str.lines().collect();

            // For small files, just show entire content with conflict markers
            if ours_lines.len() < 10 && theirs_lines.len() < 10 {
                conflict_content.push_str("<<<<<<< OURS\n");
                conflict_content.push_str(&ours_str);
                conflict_content.push_str("=======\n");
                conflict_content.push_str(&theirs_str);
                conflict_content.push_str(">>>>>>> THEIRS\n");
            } else {
                // For larger files, try to pinpoint the differences
                let mut diff_ours = String::new();
                let mut diff_theirs = String::new();
                let mut conflict_found = false;

                // Simple line-by-line comparison to find differences
                let max_len = std::cmp::max(ours_lines.len(), theirs_lines.len());
                for i in 0..max_len {
                    let ours_line = ours_lines.get(i).map_or("", |&s| s);
                    let theirs_line = theirs_lines.get(i).map_or("", |&s| s);

                    if ours_line != theirs_line {
                        // Collect a context window
                        let start = if i > 3 { i - 3 } else { 0 };
                        let end = std::cmp::min(i + 3, max_len);

                        if !conflict_found {
                            conflict_found = true;
                            
                            // Add context header
                            conflict_content.push_str(&format!("// Context around line {}\n", i + 1));
                            
                            // Start conflict section
                            conflict_content.push_str("<<<<<<< OURS\n");
                            
                            // Add context lines before conflict
                            for j in start..i {
                                if j < ours_lines.len() {
                                    diff_ours.push_str(ours_lines[j]);
                                    diff_ours.push('\n');
                                }
                            }
                        }

                        // Add differing lines
                        if i < ours_lines.len() {
                            diff_ours.push_str(ours_lines[i]);
                            diff_ours.push('\n');
                        }
                        
                        if i < theirs_lines.len() {
                            diff_theirs.push_str(theirs_lines[i]);
                            diff_theirs.push('\n');
                        }

                        // If we're at the end of the range or files, close the conflict section
                        if i == max_len - 1 || i == end - 1 {
                            conflict_content.push_str(&diff_ours);
                            conflict_content.push_str("=======\n");
                            conflict_content.push_str(&diff_theirs);
                            conflict_content.push_str(">>>>>>> THEIRS\n\n");
                            
                            // Reset for next conflict
                            diff_ours.clear();
                            diff_theirs.clear();
                            conflict_found = false;
                        }
                    } else if conflict_found {
                        // Add matching lines in both versions
                        diff_ours.push_str(ours_line);
                        diff_ours.push('\n');
                        diff_theirs.push_str(theirs_line);
                        diff_theirs.push('\n');
                        
                        // If we're at the end of the conflict context window
                        let end = std::cmp::min(i + 3, max_len);
                        if i == end - 1 {
                            conflict_content.push_str(&diff_ours);
                            conflict_content.push_str("=======\n");
                            conflict_content.push_str(&diff_theirs);
                            conflict_content.push_str(">>>>>>> THEIRS\n\n");
                            
                            // Reset for next conflict
                            diff_ours.clear();
                            diff_theirs.clear();
                            conflict_found = false;
                        }
                    }
                }
                
                // If we ended in a conflict state, close it
                if conflict_found {
                    conflict_content.push_str(&diff_ours);
                    conflict_content.push_str("=======\n");
                    conflict_content.push_str(&diff_theirs);
                    conflict_content.push_str(">>>>>>> THEIRS\n");
                }
                
                // If no conflicts were detected through comparison, fall back to full file diff
                if conflict_content.is_empty() {
                    conflict_content.push_str("<<<<<<< OURS\n");
                    conflict_content.push_str(&ours_str);
                    conflict_content.push_str("=======\n");
                    conflict_content.push_str(&theirs_str);
                    conflict_content.push_str(">>>>>>> THEIRS\n");
                }
            }
        }

        // Write to the workspace
        workspace.write_file(path, conflict_content.as_bytes())?;

        Ok(())
    }
    
    // Open editor for a file
    fn open_editor(path: &Path, editor_cmd: &str) -> Result<(), Error> {
        println!("Opening {} in editor...", path.display());
        
        // Split the editor command to handle cases with arguments
        let parts: Vec<&str> = editor_cmd.split_whitespace().collect();
        
        if parts.is_empty() {
            return Err(Error::Generic("Invalid editor command".to_string()));
        }
        
        let mut command = Command::new(parts[0]);
        
        // Add any arguments from the editor command
        for arg in &parts[1..] {
            command.arg(arg);
        }
        
        // Add the file path as the last argument
        command.arg(path);
        
        // Run the editor command
        let status = command
            .status()
            .map_err(|e| Error::Generic(format!("Failed to run editor: {}", e)))?;
        
        if !status.success() {
            return Err(Error::Generic(format!("Editor exited with status: {}", status)));
        }
        
        Ok(())
    }
    
    // Check if a conflict file has been resolved
    fn is_conflict_resolved(path: &Path, workspace: &Workspace) -> Result<bool, Error> {
        // Check if file exists
        let full_path = workspace.root_path.join(path);
        if !full_path.exists() {
            // If the file doesn't exist, we need to determine if this is intentional (resolved by deletion)
            // or the file was never created (error condition)
            return Ok(true); // Consider non-existent files as resolved - user might have intentionally deleted it
        }
        
        // Check if path is a directory
        if full_path.is_dir() {
            return Err(Error::Generic(format!("Path is a directory, not a file: {}", path.display())));
        }
        
        // Read the file content
        let content = match workspace.read_file(path) {
            Ok(content) => content,
            Err(e) => {
                // Handle error reading file (permission denied, etc.)
                return Err(Error::Generic(format!("Failed to read file: {} - {}", path.display(), e)));
            }
        };
        
        let content_str = String::from_utf8_lossy(&content);
        
        // Check for any conflict markers
        let has_ours_marker = content_str.contains("<<<<<<< OURS") || 
                             content_str.contains("<<<<<<< OURS (file doesn't exist)") ||
                             content_str.contains("<<<<<<<");
        
        let has_theirs_marker = content_str.contains(">>>>>>> THEIRS") || 
                               content_str.contains(">>>>>>> THEIRS (file doesn't exist)") ||
                               content_str.contains(">>>>>>>");
        
        let has_equals_marker = content_str.contains("=======");
        let has_base_marker = content_str.contains("||||||| BASE") || content_str.contains("|||||||");
        
        // All markers must be gone to consider the conflict resolved
        Ok(!has_ours_marker && !has_theirs_marker && !has_equals_marker && !has_base_marker)
    }
    
    // New function to find the actual file conflicts inside a directory
    fn find_directory_conflict_files(
        workspace: &Workspace,
        database: &mut Database,
        dir_path: &Path,
        base_oid: Option<&str>,
        ours_oid: Option<&str>,
        theirs_oid: Option<&str>
    ) -> Result<Vec<(PathBuf, ConflictInfo)>, Error> {
        let mut result = Vec::new();
        
        // Helper function to get files from a tree object
        fn get_files_from_tree(
            database: &mut Database,
            tree_oid: &str,
            prefix: &Path
        ) -> Result<HashMap<PathBuf, String>, Error> {
            let mut files = HashMap::new();
            
            // Load the tree object
            let obj = database.load(tree_oid)?;
            let tree = match obj.as_any().downcast_ref::<crate::core::database::tree::Tree>() {
                Some(t) => t,
                None => return Err(Error::Generic(format!("Object {} is not a tree", tree_oid)))
            };
            
            // Process each entry
            for (name, entry) in tree.get_entries() {
                let path = prefix.join(name);
                
                match entry {
                    crate::core::database::tree::TreeEntry::Blob(oid, mode) => {
                        if mode.is_directory() {
                            // This is a subtree, recurse into it
                            let subtree_obj = database.load(&oid)?;
                            if let Some(subtree) = subtree_obj.as_any().downcast_ref::<crate::core::database::tree::Tree>() {
                                if let Some(subtree_oid) = subtree.get_oid() {
                                    let subfiles = get_files_from_tree(database, subtree_oid, &path)?;
                                    files.extend(subfiles);
                                }
                            }
                        } else {
                            // This is a regular file
                            files.insert(path, oid.to_string());
                        }
                    },
                    crate::core::database::tree::TreeEntry::Tree(subtree) => {
                        if let Some(subtree_oid) = subtree.get_oid() {
                            let subfiles = get_files_from_tree(database, subtree_oid, &path)?;
                            files.extend(subfiles);
                        }
                    }
                }
            }
            
            Ok(files)
        }
        
        // Helper function to check if content is equivalent
        fn compare_file_content(
            database: &mut Database,
            oid1: Option<&str>,
            oid2: Option<&str>,
            workspace: &Workspace,
            path: &Path
        ) -> Result<bool, Error> {
            match (oid1, oid2) {
                (Some(oid1), Some(oid2)) => {
                    // If OIDs are identical, content is identical
                    if oid1 == oid2 {
                        return Ok(true);
                    }
                    
                    // Load both objects and compare content
                    let obj1 = database.load(oid1)?;
                    let obj2 = database.load(oid2)?;
                    let content1 = obj1.to_bytes();
                    let content2 = obj2.to_bytes();
                    
                    Ok(content1 == content2)
                },
                (Some(oid), None) => {
                    // Check if file exists in workspace
                    let full_path = workspace.root_path.join(path);
                    if !full_path.exists() || full_path.is_dir() {
                        return Ok(false); // Different: one exists, one doesn't
                    }
                    
                    // Compare database content with workspace content
                    let obj = database.load(oid)?;
                    let db_content = obj.to_bytes();
                    
                    match std::fs::read(&full_path) {
                        Ok(ws_content) => Ok(db_content == ws_content),
                        Err(_) => Ok(false) // Error reading workspace file, assume different
                    }
                },
                (None, Some(oid)) => {
                    // Same as above but reversed
                    let full_path = workspace.root_path.join(path);
                    if !full_path.exists() || full_path.is_dir() {
                        return Ok(false);
                    }
                    
                    let obj = database.load(oid)?;
                    let db_content = obj.to_bytes();
                    
                    match std::fs::read(&full_path) {
                        Ok(ws_content) => Ok(db_content == ws_content),
                        Err(_) => Ok(false)
                    }
                },
                (None, None) => {
                    // Both don't exist in database, check workspace
                    let full_path = workspace.root_path.join(path);
                    Ok(!full_path.exists() || full_path.is_dir())
                }
            }
        }
        
        // Get files from each version
        let base_files = if let Some(oid) = base_oid {
            get_files_from_tree(database, oid, dir_path)?
        } else {
            HashMap::new()
        };
        
        let ours_files = if let Some(oid) = ours_oid {
            get_files_from_tree(database, oid, dir_path)?
        } else {
            HashMap::new()
        };
        
        let theirs_files = if let Some(oid) = theirs_oid {
            get_files_from_tree(database, oid, dir_path)?
        } else {
            HashMap::new()
        };
        
        // Find all unique file paths
        let mut all_paths = HashSet::new();
        for path in base_files.keys() { all_paths.insert(path.clone()); }
        for path in ours_files.keys() { all_paths.insert(path.clone()); }
        for path in theirs_files.keys() { all_paths.insert(path.clone()); }
        
        println!("  Found {} unique files in directory tree", all_paths.len());
        
        // Check each path for conflicts
        for path in all_paths {
            let base_oid = base_files.get(&path).cloned();
            let ours_oid = ours_files.get(&path).cloned();
            let theirs_oid = theirs_files.get(&path).cloned();
            
            // Check for physical file existence if not in index
            let ours_exists = if ours_oid.is_none() {
                workspace.root_path.join(&path).exists()
            } else {
                true
            };
            
            let theirs_exists = theirs_oid.is_some();
            
            // Skip unless both versions exist with different content
            if !ours_exists && !theirs_exists {
                continue; // Both don't exist, no conflict
            }
            
            // Compare content
            let is_content_same = match compare_file_content(
                database,
                ours_oid.as_deref(),
                theirs_oid.as_deref(),
                workspace,
                &path
            ) {
                Ok(same) => same,
                Err(e) => {
                    println!("  Error comparing content for {}: {}", path.display(), e);
                    false // Error comparing content, assume different
                }
            };
            
            // Only count as a conflict if content differs and both versions exist
            if is_content_same || (!ours_exists && !theirs_exists) {
                println!("  Skipping file with identical content or nonexistent: {}", path.display());
                continue;
            }
            
            // Additional check - if the file doesn't physically exist but is in the index
            // with the same OID, it's not actually a conflict
            if ours_oid.is_some() && theirs_oid.is_some() && ours_oid == theirs_oid {
                println!("  Skipping file with same OID in both branches: {}", path.display());
                continue;
            }
            
            println!("  Confirmed conflict in file: {}", path.display());
            
            // Create conflict info for this file
            let conflict_info = ConflictInfo {
                path_str: path.to_string_lossy().to_string(),
                path: path.clone(),
                base_oid,
                ours_oid,
                theirs_oid,
            };
            
            result.push((path, conflict_info));
        }
        
        Ok(result)
    }
}