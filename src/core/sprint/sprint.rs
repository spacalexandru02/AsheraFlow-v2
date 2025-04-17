use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use serde::{Serialize, Deserialize};

use crate::errors::error::Error;

// Task status enum
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

impl Default for TaskStatus {
    fn default() -> Self {
        TaskStatus::Todo
    }
}

// Task structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub story_points: Option<u32>,
    pub status: TaskStatus,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub commits: Vec<String>,
}

impl Task {
    pub fn new(id: String, description: String, story_points: Option<u32>) -> Self {
        Task {
            id,
            description,
            story_points,
            status: TaskStatus::Todo,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            started_at: None,
            completed_at: None,
            commits: Vec::new(),
        }
    }
}

// Sprint structure
#[derive(Debug, Serialize, Deserialize)]
pub struct Sprint {
    pub name: String,
    pub start_date: u64,
    pub end_date: u64,
    pub tasks: HashMap<String, Task>,
    pub branch: String,
    pub total_story_points: u32,
    pub completed_story_points: u32,
}

impl Sprint {
    pub fn new(name: String, duration_days: u32) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let end_date = now + (duration_days as u64 * 24 * 60 * 60);
        let branch_name = format!("sprint-{}", name.replace(" ", "-").to_lowercase());

        Sprint {
            name,
            start_date: now,
            end_date,
            tasks: HashMap::new(),
            branch: branch_name,
            total_story_points: 0,
            completed_story_points: 0,
        }
    }

    pub fn add_task(&mut self, task: Task) -> Result<(), Error> {
        // Check if task with same ID already exists
        if self.tasks.contains_key(&task.id) {
            return Err(Error::Generic(format!("Task with ID {} already exists", task.id)));
        }

        // Update total story points
        if let Some(points) = task.story_points {
            self.total_story_points += points;
        }

        // Add task to map
        self.tasks.insert(task.id.clone(), task);

        Ok(())
    }

    pub fn complete_task(&mut self, task_id: &str) -> Result<(), Error> {
        let task = self.tasks.get_mut(task_id)
            .ok_or_else(|| Error::Generic(format!("Task with ID {} not found", task_id)))?;

        if task.status == TaskStatus::Done {
            return Err(Error::Generic(format!("Task {} is already completed", task_id)));
        }

        if task.status == TaskStatus::Todo {
            return Err(Error::Generic(format!("Task {} has not been started yet", task_id)));
        }

        task.status = TaskStatus::Done;
        task.completed_at = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        // Update completed story points
        if let Some(points) = task.story_points {
            self.completed_story_points += points;
        }

        Ok(())
    }

    pub fn get_task_duration(&self, task_id: &str) -> Result<Option<Duration>, Error> {
        let task = self.tasks.get(task_id)
            .ok_or_else(|| Error::Generic(format!("Task with ID {} not found", task_id)))?;
        
        match (task.started_at, task.completed_at) {
            (Some(start), Some(end)) => Ok(Some(Duration::from_secs(end - start))),
            _ => Ok(None)
        }
    }

    pub fn is_active(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        now <= self.end_date
    }

    pub fn format_date(timestamp: u64) -> String {
        let dt = chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .unwrap_or_else(|| chrono::DateTime::UNIX_EPOCH);
        
        dt.format("%Y-%m-%d %H:%M").to_string()
    }

    pub fn get_progress_percentage(&self) -> f32 {
        if self.total_story_points == 0 {
            return 0.0;
        }
        
        (self.completed_story_points as f32 / self.total_story_points as f32) * 100.0
    }
}

// Sprint Manager to handle storage and loading
pub struct SprintManager {
    pub repo_path: PathBuf,
}

impl SprintManager {
    pub fn new(repository_path: &Path) -> Self {
        SprintManager { repo_path: repository_path.to_path_buf() }
    }
    
    pub fn init(&self) -> Result<(), Error> {
        Ok(())
    }
    
    pub fn get_current_sprint(&self) -> Result<Option<Sprint>, Error> {
        // Create branch metadata manager
        let branch_manager = crate::core::branch_metadata::BranchMetadataManager::new(&self.repo_path);
        
        // Get current branch
        let current_branch = branch_manager.get_current_branch()?;
        
        // Check if the current branch is a sprint branch
        if !current_branch.starts_with("sprint-") {
            return Ok(None);
        }

        // Get branch metadata
        if let Ok(Some(metadata)) = branch_manager.get_sprint_metadata(&current_branch) {
            if !metadata.is_active() {
                return Ok(None);
            }

            // Create sprint from metadata
            let sprint = Sprint {
                name: metadata.name.clone(),
                start_date: metadata.start_timestamp,
                end_date: metadata.end_timestamp(),
                tasks: HashMap::new(),
                branch: current_branch,
                total_story_points: 0,
                completed_story_points: 0,
            };
            
            return Ok(Some(sprint));
        }
        
        Ok(None)
    }
    
    pub fn save_sprint(&self, sprint: &Sprint) -> Result<(), Error> {
        // Create branch metadata manager
        let branch_manager = crate::core::branch_metadata::BranchMetadataManager::new(&self.repo_path);
        
        // Create metadata
        let metadata = crate::core::branch_metadata::SprintMetadata {
            name: sprint.name.clone(),
            start_timestamp: sprint.start_date,
            duration_days: ((sprint.end_date - sprint.start_date) / 86400) as u32,
        };
        
        // Extract the branch name without the sprint- prefix if present
        let branch_name = if sprint.branch.starts_with("sprint-") {
            sprint.branch.strip_prefix("sprint-").unwrap_or(&sprint.branch).to_string()
        } else {
            sprint.branch.clone()
        };
        
        println!("[DEBUG] Saving sprint with branch: {}", branch_name);
        
        // Store metadata
        branch_manager.store_sprint_metadata(&branch_name, &metadata)?;
        
        // Now save tasks as task metadata objects
        let task_manager = crate::core::commit_metadata::CommitMetadataManager::new(&self.repo_path);
        
        for (id, task) in &sprint.tasks {
            // Convert Sprint Task to TaskMetadata
            let task_metadata = crate::core::commit_metadata::TaskMetadata {
                id: task.id.clone(),
                description: task.description.clone(),
                story_points: task.story_points,
                status: match task.status {
                    TaskStatus::Todo => crate::core::commit_metadata::TaskStatus::Todo,
                    TaskStatus::InProgress => crate::core::commit_metadata::TaskStatus::InProgress,
                    TaskStatus::Done => crate::core::commit_metadata::TaskStatus::Done,
                },
                created_at: task.created_at,
                started_at: task.started_at,
                completed_at: task.completed_at,
                commit_ids: task.commits.clone(),
            };
            
            // Store task metadata
            task_manager.store_task_metadata(&task_metadata)?;
        }
        
        Ok(())
    }
    
    pub fn has_active_sprint(&self) -> Result<bool, Error> {
        // Create branch metadata manager
        let branch_manager = crate::core::branch_metadata::BranchMetadataManager::new(&self.repo_path);
        
        // Check if there's an active sprint
        match branch_manager.find_active_sprint()? {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    
    pub fn get_sprint_tasks(&self, sprint_branch: &str) -> Result<HashMap<String, Task>, Error> {
        // Get all tasks that belong to this sprint
        let task_manager = crate::core::commit_metadata::CommitMetadataManager::new(&self.repo_path);
        let mut tasks = HashMap::new();
        
        // Need to use the task manager to list all available tasks
        let all_tasks = match task_manager.list_all_tasks() {
            Ok(tasks) => tasks,
            Err(e) => return Err(Error::Generic(format!("Failed to list tasks: {}", e))),
        };
        
        // Filter the tasks that belong to this sprint branch
        for task_metadata in all_tasks {
            // Get the task branch name
            let task_branch = format!("{}-task-{}", sprint_branch, task_metadata.id);
            
            // Check if the branch exists
            let refs = crate::core::refs::Refs::new(&self.repo_path.join(".ash"));
            let branch_exists = refs.read_ref(&format!("heads/{}", task_branch)).is_ok();
            
            if branch_exists {
                // Convert TaskMetadata to Task
                let task = Task {
                    id: task_metadata.id,
                    description: task_metadata.description,
                    story_points: task_metadata.story_points,
                    status: match task_metadata.status {
                        crate::core::commit_metadata::TaskStatus::Todo => TaskStatus::Todo,
                        crate::core::commit_metadata::TaskStatus::InProgress => TaskStatus::InProgress,
                        crate::core::commit_metadata::TaskStatus::Done => TaskStatus::Done,
                    },
                    created_at: task_metadata.created_at,
                    started_at: task_metadata.started_at,
                    completed_at: task_metadata.completed_at,
                    commits: task_metadata.commit_ids,
                };
                
                tasks.insert(task.id.clone(), task);
            }
        }
        
        Ok(tasks)
    }
} 