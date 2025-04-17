use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;
use regex::Regex;

use crate::core::database::database::Database;
use crate::core::database::commit::Commit as DatabaseCommit;
use crate::core::index::index::Index;
use crate::core::refs::Refs;
use crate::core::repository::pending_commit::{PendingCommit, PendingCommitType};
use crate::commands::commit_writer::CommitWriter;
use crate::errors::error::Error;
use crate::core::commit_metadata::{CommitMetadataManager, TaskMetadata, TaskStatus};

pub struct CommitCommand;

impl CommitCommand {
    pub fn execute(message: &str, amend: bool, reuse_message: Option<&str>, edit: bool) -> Result<(), Error> {
        let start_time = Instant::now();
        
        // Initialize repository components
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository: .ash directory not found".into()));
        }
        
        let db_path = git_path.join("objects");
        let mut database = Database::new(db_path);
        
        // Check for the index file
        let index_path = git_path.join("index");
        if !index_path.exists() {
            return Err(Error::Generic("No index file found. Please add some files first.".into()));
        }
        
        // Check for existing index.lock file before trying to load the index
        let index_lock_path = git_path.join("index.lock");
        if index_lock_path.exists() {
            return Err(Error::Generic("Another git process seems to be running in this repository.".into()));
        }
        
        let mut index = Index::new(index_path);
        
        // Load the index
        match index.load() {
            Ok(_) => println!("Index loaded successfully"),
            Err(e) => return Err(Error::Generic(format!("Error loading index: {}", e))),
        }
        
        // Check if the index is empty
        if index.entries.is_empty() {
            return Err(Error::Generic("No changes staged for commit. Use 'ash add' to add files.".into()));
        }
        
        let refs = Refs::new(&git_path);
        
        // Check if we're in a task branch
        let current_branch = match refs.current_ref() {
            Ok(reference) => match reference {
                crate::core::refs::Reference::Symbolic(path) => refs.short_name(&path),
                _ => String::new(), // Detached HEAD state
            },
            Err(_) => String::new(),
        };
        
        // Extract task ID from branch or commit message
        let mut task_id = None;
        
        // 1. Try to extract from branch name first
        if current_branch.contains("-task-") {
            let parts: Vec<&str> = current_branch.split("-task-").collect();
            if parts.len() > 1 {
                task_id = Some(parts[1].to_string());
            }
        }
        
        // 2. If not found, try to extract from commit message
        if task_id.is_none() && !message.is_empty() {
            let task_regex = Regex::new(r"((?:TASK|TEST)-\d+)").unwrap_or_else(|_| Regex::new(r"").unwrap());
            if let Some(captures) = task_regex.captures(message) {
                if let Some(m) = captures.get(1) {
                    task_id = Some(m.as_str().to_string());
                }
            }
        }
        
        // Create the commit writer
        let mut commit_writer = CommitWriter::new(
            root_path,
            git_path.clone(),
            &mut database,
            &mut index,
            &refs
        );
        
        // Check if there is a pending merge or other operation
        if commit_writer.pending_commit.in_progress(PendingCommitType::Merge) {
            return commit_writer.resume_merge(PendingCommitType::Merge, get_editor_command());
        } else if commit_writer.pending_commit.in_progress(PendingCommitType::CherryPick) {
            return commit_writer.resume_merge(PendingCommitType::CherryPick, get_editor_command());
        } else if commit_writer.pending_commit.in_progress(PendingCommitType::Revert) {
            return commit_writer.resume_merge(PendingCommitType::Revert, get_editor_command());
        }
        
        // If amending, use the amend function
        if amend {
            return commit_writer.handle_amend(get_editor_command());
        }
        
        // Get the message
        let mut msg = None;
        
        if !message.is_empty() {
            msg = Some(message.to_string());
            if !edit {
                println!("Using provided message: {}", message);
            }
        } else if let Some(rev) = reuse_message {
            // Reuse message from another commit
            msg = commit_writer.reused_message(rev)?;
            if msg.is_none() {
                return Err(Error::Generic(format!("Could not get message for revision: {}", rev)));
            }
            println!("Reusing message from commit: {}", rev);
        }
        
        // If we should edit the message, or if no message was provided
        if edit || msg.is_none() {
            // Use the editor to get the message
            let edited_message = commit_writer.compose_message(get_editor_command(), msg.as_deref())?;
            
            if let Some(message_text) = edited_message {
                // Check in edited message for task ID before we mutate message_text
                if task_id.is_none() {
                    let task_regex = Regex::new(r"((?:TASK|TEST)-\d+)").unwrap_or_else(|_| Regex::new(r"").unwrap());
                    if let Some(captures) = task_regex.captures(&message_text) {
                        if let Some(m) = captures.get(1) {
                            task_id = Some(m.as_str().to_string());
                        }
                    }
                }
                
                msg = Some(message_text);
            } else {
                // If the editor returned None, abort the commit
                return Err(Error::Generic("Aborting commit due to empty commit message".to_string()));
            }
        }
        
        // Verify we have a message
        if let Some(message_text) = msg {
            if message_text.trim().is_empty() {
                return Err(Error::Generic("Aborting commit due to empty commit message".to_string()));
            }
            
            // Get the parent commit OID
            let parent = match refs.read_head() {
                Ok(p) => {
                    println!("HEAD read successfully: {:?}", p);
                    if let Some(oid) = p {
                        vec![oid]
                    } else {
                        Vec::new()
                    }
                },
                Err(e) => {
                    println!("Error reading HEAD: {:?}", e);
                    return Err(e);
                }
            };
            
            // Create and write the commit
            let commit = commit_writer.write_commit(parent, &message_text, None)?;
            
            // Print commit information
            commit_writer.print_commit(&commit)?;
            
            // Update task status if we have a task ID
            if let Some(id) = task_id.as_ref() {
                // Get task metadata manager
                let metadata_manager = CommitMetadataManager::new(root_path);
                
                // Get task metadata
                if let Ok(Some(mut task_metadata)) = metadata_manager.get_task_metadata(id) {
                    // If task is in Todo status, update it to InProgress
                    if task_metadata.status == TaskStatus::Todo {
                        task_metadata.status = TaskStatus::InProgress;
                        
                        // Make sure started_at is set
                        if task_metadata.started_at.is_none() {
                            task_metadata.started_at = Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                            );
                        }
                        
                        // Store updated task metadata
                        if let Err(e) = metadata_manager.store_task_metadata(&task_metadata) {
                            println!("Warning: Failed to update task status: {}", e);
                        } else {
                            println!("Task {} status updated to InProgress", id);
                        }
                    }
                    
                    // Add the commit to task commit list
                    if let Some(oid) = commit.get_oid() {
                        task_metadata.commit_ids.push(oid.to_string());
                        if let Err(e) = metadata_manager.store_task_metadata(&task_metadata) {
                            println!("Warning: Failed to update task commits: {}", e);
                        }
                    }
                }
            }
            
            println!("Commit completed in {:?}", start_time.elapsed());
            Ok(())
        } else {
            Err(Error::Generic("No commit message provided".to_string()))
        }
    }
}

pub fn get_editor_command() -> Option<String> {
    env::var("GIT_EDITOR")
        .or_else(|_| env::var("VISUAL"))
        .or_else(|_| env::var("EDITOR"))
        .ok()
}
