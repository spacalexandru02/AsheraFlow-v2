use std::path::Path;
use std::io::{self, Write};
use std::collections::HashMap;

use crate::errors::error::Error;
use crate::commands::checkout::CheckoutCommand;
use crate::core::branch_metadata::BranchMetadataManager;
use crate::core::sprint::sprint::SprintManager;
use crate::core::sprint::Sprint;

pub struct SprintInfoCommand;

impl SprintInfoCommand {
    pub fn execute() -> Result<(), Error> {
        // Initialize the repository path
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository: .ash directory not found".into()));
        }
        
        // Create branch metadata manager
        let branch_manager = BranchMetadataManager::new(root_path);
        
        // Get current branch
        let current_branch = branch_manager.get_current_branch()?;
        
        // Check if there's an active sprint
        let (sprint_branch, sprint_metadata) = match branch_manager.find_active_sprint()? {
            Some((branch, metadata)) => (branch, metadata),
            None => return Err(Error::Generic("No active sprint found. Start a sprint first with 'ash sprint start'.".into())),
        };
        
        // Format dates for display
        let start_date = crate::core::branch_metadata::SprintMetadata::format_date(sprint_metadata.start_timestamp);
        let end_date = crate::core::branch_metadata::SprintMetadata::format_date(sprint_metadata.end_timestamp());
        
        // Create a sprint manager to get tasks information
        let sprint_manager = SprintManager::new(root_path);
        
        // Compute the expected sprint branch name (prefixed with "sprint-")
        let expected_sprint_branch = if sprint_branch.starts_with("sprint-") {
            sprint_branch.clone()
        } else {
            format!("sprint-{}", sprint_branch)
        };
        
        // Instead of trying to get current sprint data from current branch, 
        // create a Sprint directly from the active sprint metadata we already found
        let tasks = match sprint_manager.get_sprint_tasks(&expected_sprint_branch) {
            Ok(tasks) => tasks,
            Err(_) => HashMap::new(), // Empty HashMap if tasks can't be loaded
        };
        
        // Calculate story points
        let (total_points, completed_points) = tasks.values().fold((0, 0), |(total, completed), task| {
            let task_points = task.story_points.unwrap_or(0);
            let completed_points = if task.status == crate::core::sprint::TaskStatus::Done {
                completed + task_points
            } else {
                completed
            };
            (total + task_points, completed_points)
        });
        
        // Create a sprint object with the data we have
        let current_sprint = Sprint {
            name: sprint_metadata.name.clone(),
            start_date: sprint_metadata.start_timestamp,
            end_date: sprint_metadata.end_timestamp(),
            tasks,
            branch: expected_sprint_branch.clone(),
            total_story_points: total_points,
            completed_story_points: completed_points,
        };
        
        // Calculate progress
        let progress = current_sprint.get_progress_percentage();
        
        println!("\nActive Sprint Information:");
        println!("  Name: {}", sprint_metadata.name);
        println!("  Start date: {}", start_date);
        println!("  End date: {}", end_date);
        println!("  Branch: {}", expected_sprint_branch);
        println!("  Tasks count: {}", current_sprint.tasks.len());
        println!("  Total Story Points: {}", current_sprint.total_story_points);
        println!("  Completed Story Points: {}", current_sprint.completed_story_points);
        println!("  Progress: {:.1}%", progress);
        println!("  Current branch: {}", current_branch);
        
        // If not already on the sprint branch, ask if they want to switch
        if current_branch != expected_sprint_branch {
            println!("\nYou are currently not on the sprint branch.");
            print!("Do you want to switch to the sprint branch '{}' now? (Y/n): ", expected_sprint_branch);
            
            // Ensure the prompt is displayed
            io::stdout().flush().map_err(|e| Error::Generic(format!("IO error: {}", e)))?;
            
            // Read user input
            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .map_err(|e| Error::Generic(format!("Failed to read input: {}", e)))?;
            
            // Trim and convert to lowercase for easier comparison
            let input = input.trim().to_lowercase();
            
            // Switch to sprint branch if user confirms (default is yes)
            if input.is_empty() || input == "y" || input == "yes" {
                println!("Checking out to sprint branch: {}", expected_sprint_branch);
                CheckoutCommand::execute(&expected_sprint_branch)?;
                println!("Successfully switched to sprint branch '{}'", expected_sprint_branch);
            } else {
                println!("Remaining on current branch: {}", current_branch);
                println!("To switch to the sprint branch later, use: ash checkout {}", expected_sprint_branch);
            }
        }
        
        Ok(())
    }
} 