use std::path::Path;
use std::time::Duration;

use crate::errors::error::Error;
use crate::core::sprint::sprint::SprintManager;
use crate::core::sprint::{TaskStatus, Task};
use crate::commands::checkout::CheckoutCommand;
use crate::commands::merge::MergeCommand;
use crate::core::refs::{Refs, Reference};
use crate::core::commit_metadata::{TaskMetadata, CommitMetadataManager, TaskStatus as CommitTaskStatus};
use crate::core::branch_metadata::BranchMetadataManager;

pub struct TaskCompleteCommand;

impl TaskCompleteCommand {
    pub fn execute(id: &str, auto_merge: bool) -> Result<(), Error> {
        // Initialize the repository path
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository: .ash directory not found".into()));
        }
        
        // Create branch metadata manager to check for active sprint
        let branch_manager = BranchMetadataManager::new(root_path);
        
        // Check if there's an active sprint
        let (sprint_branch, sprint_metadata) = match branch_manager.find_active_sprint()? {
            Some((branch, metadata)) => (branch, metadata),
            None => return Err(Error::Generic("No active sprint found. Start a sprint first with 'ash sprint start'.".into())),
        };
        
        // Initialize sprint manager to get the current sprint
        let sprint_manager = SprintManager::new(root_path);
        
        // We'll create a Sprint directly from the sprint metadata we found
        // instead of requiring the user to be on the sprint branch
        let branch_name = format!("sprint-{}", sprint_metadata.name.replace(" ", "-").to_lowercase());
        let mut tasks = match sprint_manager.get_sprint_tasks(&branch_name) {
            Ok(tasks) => tasks,
            Err(_) => std::collections::HashMap::new(), // Empty HashMap if tasks can't be loaded
        };
        
        // Calculate story points
        let (total_points, completed_points) = tasks.values().fold((0, 0), |(total, completed), task| {
            let task_points = task.story_points.unwrap_or(0);
            let completed_points = if task.status == TaskStatus::Done {
                completed + task_points
            } else {
                completed
            };
            (total + task_points, completed_points)
        });
        
        // Create a sprint object with the data we have
        let mut current_sprint = crate::core::sprint::Sprint {
            name: sprint_metadata.name.clone(),
            start_date: sprint_metadata.start_timestamp,
            end_date: sprint_metadata.end_timestamp(),
            tasks,
            branch: branch_name.clone(),
            total_story_points: total_points,
            completed_story_points: completed_points,
        };
        
        // Get the task and check if it's in progress
        let task = match current_sprint.tasks.get(id) {
            Some(t) => {
                if t.status != TaskStatus::InProgress {
                    return Err(Error::Generic(format!("Task {} is not in progress. Cannot complete a task that hasn't been started.", id)));
                }
                t
            },
            None => {
                // Try to get task directly from TaskMetadata system
                let task_manager = CommitMetadataManager::new(root_path);
                match task_manager.get_task_metadata(id)? {
                    Some(task_metadata) => {
                        if task_metadata.status != CommitTaskStatus::InProgress {
                            return Err(Error::Generic(format!("Task {} is not in progress. Cannot complete a task that hasn't been started.", id)));
                        }
                        
                        // Create and add a sprint task
                        let task = Task::new(
                            task_metadata.id.clone(),
                            task_metadata.description.clone(),
                            task_metadata.story_points
                        );
                        
                        // Add to sprint
                        current_sprint.add_task(task.clone())?;
                        match current_sprint.tasks.get(id) {
                            Some(t) => t,
                            None => return Err(Error::Generic(format!("Failed to add task {} to sprint", id)))
                        }
                    }
                    None => return Err(Error::Generic(format!("Task with ID {} not found in current sprint", id))),
                }
            }
        };
        
        // Construct branch names
        let expected_sprint_branch = branch_name.clone();
        let task_branch = format!("{}-task-{}", expected_sprint_branch, id);
        
        // Check if we're on the correct branch, but only to inform the user
        let refs = Refs::new(&git_path);
        let current_ref = refs.current_ref()?;
        
        let current_branch = match current_ref {
            Reference::Symbolic(path) => refs.short_name(&path),
            _ => String::new(), // Detached HEAD state
        };
        
        // Show an informational message instead of error if not on task branch
        if current_branch != task_branch {
            println!("Note: You are not on the task branch '{}'. The task will still be completed.", task_branch);
        }
        
        // Complete the task
        // If task is in Todo status, update it to InProgress first
        let mut task_metadata_manager = CommitMetadataManager::new(root_path);
        if let Ok(Some(mut task_metadata)) = task_metadata_manager.get_task_metadata(id) {
            if task_metadata.status == CommitTaskStatus::Todo {
                println!("Note: Task is in Todo status. Automatically updating to InProgress before completing.");
                task_metadata.status = CommitTaskStatus::InProgress;
                task_metadata.started_at = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                );
                task_metadata_manager.store_task_metadata(&task_metadata)?;
                
                // Also update task in current_sprint
                if let Some(task) = current_sprint.tasks.get_mut(id) {
                    task.status = TaskStatus::InProgress;
                    task.started_at = task_metadata.started_at;
                }
            }
        }
        
        current_sprint.complete_task(id)?;
        
        // Update task in task metadata system
        let task_manager = CommitMetadataManager::new(root_path);
        if let Ok(Some(mut task_metadata)) = task_manager.get_task_metadata(id) {
            task_metadata.status = CommitTaskStatus::Done;
            task_metadata.completed_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
            task_manager.store_task_metadata(&task_metadata)?;
        }
        
        // Get task duration
        let duration = match current_sprint.get_task_duration(id)? {
            Some(d) => d,
            None => Duration::from_secs(0),
        };
        
        // Get task metadata for formatting duration
        let task_metadata = task_manager.get_task_metadata(id)?.unwrap_or_else(|| {
            let mut tm = TaskMetadata::new(id.to_string(), "Unknown".to_string(), None);
            tm.status = CommitTaskStatus::Done;
            tm
        });
        
        // Save the updated sprint
        sprint_manager.save_sprint(&current_sprint)?;
        
        // Display task information
        println!("\nTask completed successfully:");
        println!("  ID: {}", id);
        println!("  Duration: {}", task_metadata.format_duration());
        
        // Display sprint progress
        println!("\nSprint progress:");
        println!("  Total Story Points: {}", current_sprint.total_story_points);
        println!("  Completed Story Points: {}", current_sprint.completed_story_points);
        println!("  Progress: {:.1}%", current_sprint.get_progress_percentage());
        
        // Handle auto merge if requested
        if auto_merge {
            println!("\nPerforming auto-merge to sprint branch {}...", branch_name);
            
            // Checkout the sprint branch
            println!("Switching to branch '{}'", branch_name);
            CheckoutCommand::execute(&branch_name)?;
            
            // Merge the task branch using MergeCommand
            let merge_message = format!("Merge task/{} into {}", id, branch_name);
            MergeCommand::execute(&task_branch, Some(&merge_message))?;
            
            println!("Successfully merged task branch into sprint branch");
        } else {
            // Checkout the sprint branch even if not auto-merging
            println!("\nSwitching to sprint branch '{}'", branch_name);
            CheckoutCommand::execute(&branch_name)?;
            
            println!("Task branch was not automatically merged.");
            println!("To merge the task branch manually:");
            println!("  ash merge {}", task_branch);
        }
        
        Ok(())
    }
} 