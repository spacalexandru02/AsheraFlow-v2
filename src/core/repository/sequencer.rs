use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::fmt;

use regex::Regex;

use crate::core::database::commit::Commit;
use crate::core::database::database::Database;
use crate::core::lockfile::Lockfile;
use crate::core::refs::{Refs, HEAD};
use crate::errors::error::Error;

fn get_line_regex() -> Regex {
    Regex::new(r"^(\S+) (\S+) (.*)$").unwrap()
}

/// Actions that can be performed during sequencing
#[derive(Debug, Clone, Copy)]
pub enum Action {
    Pick,
    Revert,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let action = match self {
            Action::Pick => "pick",
            Action::Revert => "revert",
        };
        write!(f, "{}", action)
    }
}

impl FromStr for Action {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pick" => Ok(Action::Pick),
            "revert" => Ok(Action::Revert),
            _ => Err(Error::Generic(format!("Unknown sequencer action: {}", s))),
        }
    }
}

/// The Sequencer handles sequenced operations like cherry-pick and revert
#[derive(Debug)]
pub struct Sequencer {
    // Base paths
    pub repo_path: PathBuf,
    pathname: PathBuf,
    abort_path: PathBuf,
    head_path: PathBuf,
    todo_path: PathBuf,
    todo_file: Option<Lockfile>,
    options_path: PathBuf,
    
    // Sequencing state
    commands: Vec<(Action, Commit)>,
}

impl Sequencer {
    /// Create a new Sequencer for the given repository
    pub fn new(repo_path: PathBuf) -> Self {
        let pathname = repo_path.join("sequencer");
        let abort_path = pathname.join("abort-safety");
        let head_path = pathname.join("head");
        let todo_path = pathname.join("todo");
        let options_path = pathname.join("opts");

        Self {
            repo_path,
            pathname,
            abort_path,
            head_path,
            todo_path,
            todo_file: None,
            options_path,
            commands: Vec::new(),
        }
    }

    /// Start a new sequencing operation
    pub fn start(&mut self, options: &HashMap<String, String>) -> Result<(), Error> {
        // Create sequencer directory if it doesn't exist
        if !self.pathname.exists() {
            fs::create_dir_all(&self.pathname)?;
        }

        // Load and store the HEAD reference
        let refs = Refs::new(&self.repo_path);
        let head_oid = refs.read_head()?.ok_or_else(|| Error::Generic("Cannot start sequencer: HEAD not found".to_string()))?;
        
        // Store the current HEAD for safety
        self.write_file(&self.head_path, &head_oid)?;
        self.write_file(&self.abort_path, &head_oid)?;

        // Store options
        let mut file = File::create(&self.options_path)?;
        for (key, value) in options {
            writeln!(file, "{}={}", key, value)?;
        }

        // Prepare todo file
        self.open_todo_file()?;

        Ok(())
    }

    /// Get an option value from the options file
    pub fn get_option(&self, name: &str) -> Result<Option<String>, Error> {
        if !self.options_path.exists() {
            return Ok(None);
        }

        let mut content = String::new();
        let mut file = File::open(&self.options_path)?;
        file.read_to_string(&mut content)?;

        for line in content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                if key == name {
                    return Ok(Some(value.to_string()));
                }
            }
        }

        Ok(None)
    }

    /// Add a cherry-pick command to the sequencer
    pub fn add_pick(&mut self, commit: Commit) {
        self.commands.push((Action::Pick, commit));
    }

    /// Add a revert command to the sequencer
    pub fn add_revert(&mut self, commit: Commit) {
        self.commands.push((Action::Revert, commit));
    }

    /// Get the next command from the sequencer
    pub fn next_command(&self) -> Option<(Action, Commit)> {
        self.commands.first().map(|(action, commit)| (action.to_owned(), commit.to_owned()))
    }

    /// Drop the current command from the sequencer
    pub fn drop_command(&mut self) -> Result<(), Error> {
        if self.commands.is_empty() {
            return Ok(());
        }
        
        self.commands.remove(0);
        
        // Update abort safety file with current HEAD
        let refs = Refs::new(&self.repo_path);
        let head_oid = refs.read_head()?.ok_or_else(|| Error::Generic("Cannot update abort safety: HEAD not found".to_string()))?;
        self.write_file(&self.abort_path, &head_oid)?;

        Ok(())
    }

    /// Load the sequencer state from disk
    pub fn load(&mut self) -> Result<(), Error> {
        self.open_todo_file()?;

        if !self.todo_path.exists() {
            return Ok(());
        }

        let mut content = String::new();
        let mut file = File::open(&self.todo_path)?;
        file.read_to_string(&mut content)?;

        self.commands.clear();
        let mut database = Database::new(self.repo_path.join("objects"));
        let line_regex = get_line_regex();

        for line in content.lines() {
            if let Some(captures) = line_regex.captures(line) {
                let action = &captures[1];
                let oid = &captures[2];
                
                // Load the commit object
                let obj = database.load(oid)?;
                let commit = match obj.as_any().downcast_ref::<Commit>() {
                    Some(commit) => commit.clone(),
                    None => return Err(Error::Generic(format!("Invalid commit object: {}", oid)))
                };
                
                // Add the command to the queue
                self.commands.push((Action::from_str(action)?, commit));
            }
        }

        Ok(())
    }

    /// Save the current sequencer state to disk
    pub fn dump(&mut self) -> Result<(), Error> {
        if let Some(todo_file) = &mut self.todo_file {
            let mut database = Database::new(self.repo_path.join("objects"));
            
            for (action, commit) in &self.commands {
                let oid = commit.get_oid().map_or_else(String::new, |s| s.clone());
                let short = database.short_oid(&oid);
                todo_file.write(&format!("{} {} {}\n", action, short, commit.title_line()))
                    .map_err(|e| Error::Generic(format!("Failed to write to todo file: {:?}", e)))?;
            }

            todo_file.commit_ref()
                .map_err(|e| Error::Generic(format!("Failed to commit todo file: {:?}", e)))?;
        }

        Ok(())
    }

    /// Abort the current sequencing operation
    pub fn abort(&mut self) -> Result<(), Error> {
        // Load the original HEAD
        let head_oid = fs::read_to_string(&self.head_path)?.trim().to_owned();
        
        // Load the safety point
        let expected = fs::read_to_string(&self.abort_path)?.trim().to_owned();
        
        // Check current HEAD
        let refs = Refs::new(&self.repo_path);
        let actual = refs.read_head()?.unwrap_or_else(String::new);

        // Clean up sequencer files
        self.quit()?;

        // Verify it's safe to reset
        if actual != expected {
            return Err(Error::Generic("Cannot abort: Working directory has been modified since last command".to_string()));
        }

        // Reset to original HEAD
        self.hard_reset(&head_oid)?;

        Ok(())
    }

    /// Perform a hard reset to the given commit
    fn hard_reset(&self, commit_oid: &str) -> Result<(), Error> {
        // Reset HEAD
        let refs = Refs::new(&self.repo_path);
        refs.update_head(commit_oid)?;
        
        // TODO: Reset working directory and index
        // This would need to be implemented

        Ok(())
    }

    /// Quit the current sequencing operation
    pub fn quit(&self) -> Result<(), Error> {
        if self.pathname.exists() {
            fs::remove_dir_all(&self.pathname)?;
        }

        Ok(())
    }

    /// Helper to write a string to a file
    fn write_file(&self, path: &Path, content: &str) -> Result<(), Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| Error::Generic(format!("Failed to open file: {}", e)))?;
        
        writeln!(file, "{}", content)
            .map_err(|e| Error::Generic(format!("Failed to write to file: {}", e)))?;

        Ok(())
    }

    /// Open the todo file for writing
    fn open_todo_file(&mut self) -> Result<(), Error> {
        if !self.pathname.exists() {
            return Ok(());
        }

        self.todo_file = Some(Lockfile::new(self.todo_path.clone()));
        if let Some(todo_file) = &mut self.todo_file {
            todo_file.hold_for_update()
                .map_err(|e| Error::Generic(format!("Failed to lock todo file: {:?}", e)))?;
        }

        Ok(())
    }
} 