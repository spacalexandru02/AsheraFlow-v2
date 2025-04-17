use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use regex::Regex;
use crate::errors::error::Error;
use crate::core::database::database::Database;
use crate::core::refs::Refs;
use crate::core::database::commit::Commit;
use crate::core::repository::repository::Repository;
use crate::core::database::task_metadata_object::TaskMetadataObject;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

#[derive(Debug, Clone)]
pub struct TaskMetadata {
    pub id: String,
    pub description: String,
    pub story_points: Option<u32>,
    pub status: TaskStatus,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub commit_ids: Vec<String>,
}

impl TaskMetadata {
    pub fn new(id: String, description: String, story_points: Option<u32>) -> Self {
        TaskMetadata {
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
            commit_ids: Vec::new(),
        }
    }

    // Generate a commit message format for tasks
    pub fn format_commit_message(&self, message: &str) -> String {
        format!("[Task-{}] {}", self.id, message)
    }

    // Extract task metadata from commit message
    pub fn extract_from_message(message: &str) -> Option<String> {
        lazy_static::lazy_static! {
            static ref TASK_REGEX: Regex = Regex::new(r"^\[Task-([^\]]+)\]").unwrap();
        }
        
        if let Some(captures) = TASK_REGEX.captures(message) {
            if let Some(task_id) = captures.get(1) {
                return Some(task_id.as_str().to_string());
            }
        }
        
        None
    }
    
    // Calculate duration if task is completed
    pub fn get_duration(&self) -> Option<Duration> {
        match (self.started_at, self.completed_at) {
            (Some(start), Some(end)) => Some(Duration::from_secs(end - start)),
            _ => None,
        }
    }
    
    // Format duration to string
    pub fn format_duration(&self) -> String {
        if let Some(duration) = self.get_duration() {
            let hours = duration.as_secs() / 3600;
            let minutes = (duration.as_secs() % 3600) / 60;
            format!("{}h {}m", hours, minutes)
        } else {
            "Not completed".to_string()
        }
    }
    
    // Encode task metadata as a special comment in commit messages
    pub fn encode(&self) -> String {
        let story_points_str = match self.story_points {
            Some(sp) => sp.to_string(),
            None => "".to_string(),
        };
        
        let started_at_str = match self.started_at {
            Some(ts) => ts.to_string(),
            None => "".to_string(),
        };
        
        let completed_at_str = match self.completed_at {
            Some(ts) => ts.to_string(),
            None => "".to_string(),
        };
        
        let status_str = match self.status {
            TaskStatus::Todo => "TODO",
            TaskStatus::InProgress => "IN_PROGRESS",
            TaskStatus::Done => "DONE",
        };
        
        format!(
            "TASK-METADATA:{}:{}:{}:{}:{}:{}:{}",
            self.id,
            self.description,
            story_points_str,
            status_str,
            self.created_at,
            started_at_str,
            completed_at_str
        )
    }
    
    // Decode task metadata from encoded string
    pub fn decode(encoded: &str) -> Option<Self> {
        let parts: Vec<&str> = encoded.split(':').collect();
        if parts.len() >= 8 && parts[0] == "TASK-METADATA" {
            let id = parts[1].to_string();
            let description = parts[2].to_string();
            
            let story_points = if parts[3].is_empty() {
                None
            } else {
                parts[3].parse::<u32>().ok()
            };
            
            let status = match parts[4] {
                "TODO" => TaskStatus::Todo,
                "IN_PROGRESS" => TaskStatus::InProgress,
                "DONE" => TaskStatus::Done,
                _ => TaskStatus::Todo,
            };
            
            let created_at = parts[5].parse::<u64>().ok()?;
            
            let started_at = if parts[6].is_empty() {
                None
            } else {
                parts[6].parse::<u64>().ok()
            };
            
            let completed_at = if parts[7].is_empty() {
                None
            } else {
                parts[7].parse::<u64>().ok()
            };
            
            Some(TaskMetadata {
                id,
                description,
                story_points,
                status,
                created_at,
                started_at,
                completed_at,
                commit_ids: Vec::new(),
            })
        } else {
            None
        }
    }
}

pub struct CommitMetadataManager {
    repo_path: PathBuf,
}

impl CommitMetadataManager {
    pub fn new(repo_path: &Path) -> Self {
        CommitMetadataManager {
            repo_path: repo_path.to_path_buf(),
        }
    }
    
    // Format task metadata for inclusion in commit messages
    pub fn format_task_metadata(&self, task: &TaskMetadata) -> String {
        format!("Task: {}\nStatus: {}", 
            task.id,
            match task.status {
                TaskStatus::Todo => "TODO",
                TaskStatus::InProgress => "IN_PROGRESS",
                TaskStatus::Done => "DONE",
            }
        )
    }
    
    // Store task metadata in the object database
    pub fn store_task_metadata(&self, task: &TaskMetadata) -> Result<(), Error> {
        // Creăm un repository și avem acces la database
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let mut repo = Repository::new(repo_str)?;
        
        // Creăm obiectul de metadate și îl stocăm în database
        let mut meta_obj = TaskMetadataObject::new(task.clone());
        let oid = repo.database.store(&mut meta_obj)?;
        
        // Creăm o referință specială pentru metadatele task-ului
        let meta_ref = format!("refs/meta/task/{}", task.id);
        repo.refs.update_ref(&meta_ref, &oid)?;
        
        Ok(())
    }
    
    // Get task metadata by ID from the object database
    pub fn get_task_metadata(&self, task_id: &str) -> Result<Option<TaskMetadata>, Error> {
        // Creăm un repository și avem acces la database
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let mut repo = Repository::new(repo_str)?;
        
        // Citim referința pentru metadate
        let meta_ref = format!("refs/meta/task/{}", task_id);
        let oid = match repo.refs.read_ref(&meta_ref)? {
            Some(oid) => oid,
            None => return Ok(None),
        };
        
        // Încărcăm obiectul din database
        let obj = repo.database.load(&oid)?;
        if let Some(meta_obj) = obj.as_any().downcast_ref::<TaskMetadataObject>() {
            Ok(Some(meta_obj.get_metadata().clone()))
        } else {
            Err(Error::Generic("Invalid metadata object type".into()))
        }
    }
    
    // Find all tasks related to a sprint (based on branch)
    pub fn find_sprint_tasks(&self, sprint_branch: &str) -> Result<Vec<TaskMetadata>, Error> {
        // Creăm un repository și avem acces la database
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let repo = Repository::new(repo_str)?;
        
        let mut tasks = Vec::new();
        
        // Obținem toate referințele meta/task/*
        let refs = repo.refs.list_refs_with_prefix("refs/meta/task/")?;
            
        // First load all task metadata
        for reference in refs {
            match reference {
                crate::core::refs::Reference::Symbolic(path) => {
                    let task_id = path.strip_prefix("refs/meta/task/")
                        .unwrap_or(&path)
                        .to_string();
                    
                    if let Ok(Some(mut task)) = self.get_task_metadata(&task_id) {
                        // Find all commits for this task and check if they belong to the sprint branch
                        self.populate_task_commits(&mut task, sprint_branch)?;
                        
                        if !task.commit_ids.is_empty() {
                            tasks.push(task);
                        }
                    }
                },
                _ => continue,
            }
        }
        
        // Sort by created date
        tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        
        Ok(tasks)
    }
    
    // Populate task with its commits
    fn populate_task_commits(&self, task: &mut TaskMetadata, branch: &str) -> Result<(), Error> {
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let repo = Repository::new(repo_str)?;
        
        let db_path = self.repo_path.join(".ash").join("objects");
        let mut database = Database::new(db_path);
        
        // Get all commits in the branch
        let refs = Refs::new(&self.repo_path.join(".ash"));
        
        // Simple approach - check each commit in the branch
        if let Ok(Some(current_oid)) = refs.read_ref(branch) {
            let mut current = current_oid;
            
            while !current.is_empty() {
                if let Ok(object) = database.load(&current) {
                    if let Some(commit) = object.as_any().downcast_ref::<Commit>() {
                        let message = commit.get_message();
                        
                        // Check if this commit belongs to our task
                        if let Some(commit_task_id) = TaskMetadata::extract_from_message(&message) {
                            if commit_task_id == task.id {
                                task.commit_ids.push(current.clone());
                            }
                        }
                        
                        // Move to parent commit
                        if let Some(parent) = commit.get_parent() {
                            current = parent.to_string();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        
        Ok(())
    }
    
    // Calculate sprint progress
    pub fn calculate_sprint_progress(&self, tasks: &[TaskMetadata]) -> (u32, u32, f32) {
        let mut total_points = 0;
        let mut completed_points = 0;
        
        for task in tasks {
            if let Some(points) = task.story_points {
                total_points += points;
                
                if task.status == TaskStatus::Done {
                    completed_points += points;
                }
            }
        }
        
        let progress_percentage = if total_points > 0 {
            (completed_points as f32 / total_points as f32) * 100.0
        } else {
            0.0
        };
        
        (total_points, completed_points, progress_percentage)
    }
    
    // Create branch name for a task
    pub fn create_task_branch_name(&self, sprint_branch: &str, task_id: &str) -> String {
        format!("{}/task/{}", sprint_branch, task_id)
    }
    
    // List all tasks in the repository
    pub fn list_all_tasks(&self) -> Result<Vec<TaskMetadata>, Error> {
        // Create a repository and get access to references
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let repo = Repository::new(repo_str)?;
        
        let mut tasks = Vec::new();
        
        // Get all references with meta/task/ prefix
        let refs = repo.refs.list_refs_with_prefix("refs/meta/task/")?;
            
        // Load all task metadata
        for reference in refs {
            match reference {
                crate::core::refs::Reference::Symbolic(path) => {
                    let task_id = path.strip_prefix("refs/meta/task/")
                        .unwrap_or(&path)
                        .to_string();
                    
                    if let Ok(Some(task)) = self.get_task_metadata(&task_id) {
                        tasks.push(task);
                    }
                },
                _ => continue,
            }
        }
        
        // Sort by creation date
        tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        
        Ok(tasks)
    }
} 