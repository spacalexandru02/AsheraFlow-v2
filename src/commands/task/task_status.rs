use std::path::Path;

use crate::errors::error::Error;
use crate::core::commit_metadata::{CommitMetadataManager, TaskStatus};
use chrono::NaiveDateTime;

pub struct TaskStatusCommand;

impl TaskStatusCommand {
    pub fn execute(id: &str) -> Result<(), Error> {
        let task_manager = CommitMetadataManager::new(Path::new("."));
        
        match task_manager.get_task_metadata(id)? {
            Some(task) => {
                println!("Task {} Status:", id);
                println!("  ID: {}", task.id);
                println!("  Description: {}", task.description);
                if let Some(points) = task.story_points {
                    println!("  Story Points: {}", points);
                } else {
                    println!("  Story Points: 0");
                }
                println!("  Status: {:?}", task.status);
                
                let created_datetime = chrono::NaiveDateTime::from_timestamp_opt(task.created_at as i64, 0)
                    .unwrap_or_default();
                println!("  Created: {}", created_datetime);
                
                if let Some(started_at) = task.started_at {
                    let started_datetime = chrono::NaiveDateTime::from_timestamp_opt(started_at as i64, 0)
                        .unwrap_or_default();
                    println!("  Started: {}", started_datetime);
                }
                
                if let Some(completed_at) = task.completed_at {
                    let completed_datetime = chrono::NaiveDateTime::from_timestamp_opt(completed_at as i64, 0)
                        .unwrap_or_default();
                    println!("  Completed: {}", completed_datetime);
                }
                
                if !task.commit_ids.is_empty() {
                    println!("  Commits: {}", task.commit_ids.len());
                    for (i, commit_id) in task.commit_ids.iter().enumerate() {
                        println!("    {}: {}", i+1, commit_id);
                    }
                }
                
                println!("  Duration: {}", task.format_duration());
                
                Ok(())
            },
            None => {
                Err(Error::Generic(format!("Task with ID {} not found", id)))
            }
        }
    }
} 