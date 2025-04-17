use std::path::Path;

use crate::errors::error::Error;
use crate::commands::branch::BranchCommand;
use crate::commands::checkout::CheckoutCommand;
use crate::core::branch_metadata::{SprintMetadata, BranchMetadataManager};
use crate::core::sprint::sprint::SprintManager;

pub struct SprintStartCommand;

impl SprintStartCommand {
    pub fn execute(name: &str, duration: u32) -> Result<(), Error> {
        println!("Starting new sprint: {} (Duration: {} days)", name, duration);
        
        // Initialize the repository path
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository: .ash directory not found".into()));
        }
        
        // Create branch metadata manager
        let branch_manager = BranchMetadataManager::new(root_path);
        
        // Check if there's already an active sprint
        if let Some((active_branch, active_meta)) = branch_manager.find_active_sprint()? {
            return Err(Error::Generic(format!(
                "There is already an active sprint '{}'. Complete or cancel it before starting a new one.",
                active_meta.name
            )));
        }
        
        // Create a new sprint metadata
        let sprint_metadata = SprintMetadata::new(name.to_string(), duration);
        let branch_name = sprint_metadata.to_branch_name();
        
        // Format dates for display
        let start_date = SprintMetadata::format_date(sprint_metadata.start_timestamp);
        let end_date = SprintMetadata::format_date(sprint_metadata.end_timestamp());
        
        // Display sprint information
        println!("Sprint information:");
        println!("  Name: {}", sprint_metadata.name);
        println!("  Start date: {}", start_date);
        println!("  End date: {}", end_date);
        println!("  Branch: {}", branch_name);
        
        // Create the sprint branch
        println!("Creating sprint branch: {}", branch_name);
        
        // Create branch using BranchCommand
        match BranchCommand::execute(&branch_name, None) {
            Ok(_) => {},
            Err(e) => {
                // Skip error if branch already exists
                if !e.to_string().contains("already exists") {
                    return Err(e);
                }
                println!("Branch already exists, using existing branch.");
            }
        }
        
        // Checkout the branch
        println!("Checking out sprint branch...");
        CheckoutCommand::execute(&branch_name)?;
        println!("Successfully switched to branch '{}'", branch_name);
        
        // Store the sprint metadata
        branch_manager.store_sprint_metadata(&branch_name, &sprint_metadata)?;
        
        // Also create a Sprint object and save it via SprintManager
        // This is for backwards compatibility with existing code
        let sprint_manager = SprintManager::new(root_path);
        let sprint = crate::core::sprint::Sprint {
            name: sprint_metadata.name.clone(),
            start_date: sprint_metadata.start_timestamp,
            end_date: sprint_metadata.end_timestamp(),
            tasks: std::collections::HashMap::new(),
            branch: branch_name.clone(),
            total_story_points: 0,
            completed_story_points: 0,
        };
        sprint_manager.save_sprint(&sprint)?;
        
        println!("\nSprint '{}' started successfully!", name);
        println!("You can now create tasks with: ash task create <id> <description> [story_points]");
        println!("The sprint will end on: {}", end_date);
        
        Ok(())
    }
} 