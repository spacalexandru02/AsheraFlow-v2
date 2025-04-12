// src/commands/merge_tool.rs
use std::process::Command;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::env;
use std::io::{self, Write};
use std::collections::HashSet;

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
        
        // Collect all conflict information up front to avoid borrow issues
        let mut conflict_infos = Vec::new();
        
        // First, collect information about all conflicted entries
        {
            // This block limits the lifetime of the immutable borrow of index
            let entries = index.each_entry().collect::<Vec<_>>();
            
            for path_str in &conflicted_paths {
                let path = PathBuf::from(path_str);
                
                // Skip if file doesn't exist in workspace
                if !workspace.path_exists(&path)? {
                    println!("\nSkipping conflict in file: {} (file does not exist in workspace)", 
                             Color::yellow(path_str));
                    skipped_count += 1;
                    continue;
                }
                
                // Collect all stage entries for this path
                let mut base_oid = None;
                let mut ours_oid = None;
                let mut theirs_oid = None;
                
                for entry in &entries {
                    if entry.get_path() == path_str {
                        match entry.stage {
                            1 => base_oid = Some(entry.get_oid().to_string()),
                            2 => ours_oid = Some(entry.get_oid().to_string()),
                            3 => theirs_oid = Some(entry.get_oid().to_string()),
                            _ => {}
                        }
                    }
                }
                
                conflict_infos.push(ConflictInfo {
                    path_str: path_str.clone(),
                    path,
                    base_oid,
                    ours_oid,
                    theirs_oid
                });
            }
        }
        
        // Now process each conflicted file one by one
        for info in conflict_infos {
            let path_str = &info.path_str;
            let path = &info.path;
            
            println!("\nProcessing conflict in file: {}", Color::yellow(path_str));
            
            // Create conflict-marked file
            Self::create_conflict_file(&workspace, &mut database, path, 
                                     info.base_oid.as_deref(), 
                                     info.ours_oid.as_deref(), 
                                     info.theirs_oid.as_deref())?;
            
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
                    if let Err(e) = Self::open_editor(path, &editor) {
                        println!("Error opening editor: {}", e);
                        skipped_count += 1;
                        continue;
                    }
                    
                    // Check if conflict was resolved
                    if Self::is_conflict_resolved(path, &workspace)? {
                        // Update index with resolved file
                        let stat = workspace.stat_file(path)?;
                        let file_contents = workspace.read_file(path)?;
                        let mut blob = Blob::new(file_contents);
                        database.store(&mut blob)?;
                        let oid = blob.get_oid().unwrap().clone();
                        
                        // Resolve conflict in index
                        index.resolve_conflict(path, &oid, &stat)?;
                        println!("  {} Conflict resolved for file: {}", Color::green("✓"), path_str);
                        resolved_count += 1;
                    } else {
                        println!("  {} Conflict markers still present, conflict not resolved.", Color::red("✗"));
                        skipped_count += 1;
                    }
                },
                "2" => {
                    // Accept "ours" version
                    if let Some(oid) = info.ours_oid.as_deref() {
                        let obj = database.load(oid)?;
                        let content = obj.to_bytes();
                        workspace.write_file(path, &content)?;
                        let stat = workspace.stat_file(path)?;
                        
                        // Resolve the conflict
                        index.resolve_conflict(path, oid, &stat)?;
                        println!("  {} Accepted 'ours' version for file: {}", Color::green("✓"), path_str);
                        resolved_count += 1;
                    } else {
                        println!("  {} No 'ours' version available.", Color::red("✗"));
                        skipped_count += 1;
                    }
                },
                "3" => {
                    // Accept "theirs" version
                    if let Some(oid) = info.theirs_oid.as_deref() {
                        let obj = database.load(oid)?;
                        let content = obj.to_bytes();
                        workspace.write_file(path, &content)?;
                        let stat = workspace.stat_file(path)?;
                        
                        // Resolve the conflict
                        index.resolve_conflict(path, oid, &stat)?;
                        println!("  {} Accepted 'theirs' version for file: {}", Color::green("✓"), path_str);
                        resolved_count += 1;
                    } else {
                        println!("  {} No 'theirs' version available.", Color::red("✗"));
                        skipped_count += 1;
                    }
                },
                "4" => {
                    println!("  Skipped file: {}", path_str);
                    skipped_count += 1;
                },
                "q" | "Q" => {
                    println!("Quitting resolution process.");
                    break;
                },
                _ => {
                    println!("Invalid choice. Skipping file.");
                    skipped_count += 1;
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
        // Prepare content from all stages
        let base_content = if let Some(oid) = base_oid {
            let obj = database.load(oid)?;
            obj.to_bytes()
        } else {
            Vec::new()
        };
        
        let ours_content = if let Some(oid) = ours_oid {
            let obj = database.load(oid)?;
            obj.to_bytes()
        } else {
            Vec::new()
        };
        
        let theirs_content = if let Some(oid) = theirs_oid {
            let obj = database.load(oid)?;
            obj.to_bytes()
        } else {
            Vec::new()
        };
        
        // Try to create a clean diff3 style conflict
        let mut conflict_content = String::new();
        
        // Convert byte content to strings
        let base_str = String::from_utf8_lossy(&base_content).to_string();
        let ours_str = String::from_utf8_lossy(&ours_content).to_string();
        let theirs_str = String::from_utf8_lossy(&theirs_content).to_string();
        
        // Check if all three versions are available for a proper diff3
        if !base_str.is_empty() && !ours_str.is_empty() && !theirs_str.is_empty() {
            // If we can use diff3 format
            // Split strings into lines for diff processing
            let base_lines = diff::split_lines(&base_str);
            let ours_lines = diff::split_lines(&ours_str);
            let theirs_lines = diff::split_lines(&theirs_str);
            
            // Try to use our diff module to create a proper diff3 marked file
            // (Note: actual implementation of diff3 would be more complex)
            // For now, we'll just use simple conflict markers
            conflict_content.push_str(MERGE_MARKER_OURS_BEGIN);
            conflict_content.push_str(&ours_str);
            conflict_content.push_str(MERGE_MARKER_BASE_BEGIN);
            conflict_content.push_str(&base_str);
            conflict_content.push_str(MERGE_MARKER_MIDDLE);
            conflict_content.push_str(&theirs_str);
            conflict_content.push_str(MERGE_MARKER_THEIRS_END);
        } else {
            // Simple conflict markers without base
            conflict_content.push_str(MERGE_MARKER_OURS_BEGIN);
            conflict_content.push_str(&ours_str);
            conflict_content.push_str(MERGE_MARKER_MIDDLE);
            conflict_content.push_str(&theirs_str);
            conflict_content.push_str(MERGE_MARKER_THEIRS_END);
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
        // Read the file content
        let content = workspace.read_file(path)?;
        let content_str = String::from_utf8_lossy(&content);
        
        // Check for conflict markers
        let has_conflict_markers = content_str.contains(MERGE_MARKER_OURS_BEGIN) && 
                                   content_str.contains(MERGE_MARKER_MIDDLE) &&
                                   content_str.contains(MERGE_MARKER_THEIRS_END);
        
        // Additionally check for base marker if present
        let has_base_markers = content_str.contains(MERGE_MARKER_BASE_BEGIN);
        
        Ok(!has_conflict_markers && !has_base_markers)
    }
}