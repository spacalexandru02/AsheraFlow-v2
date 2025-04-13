use std::fs::read_to_string;
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::core::database::author::Author;
use crate::core::database::commit::Commit;
use crate::core::database::database::Database;
use crate::core::database::tree::Tree;
use crate::core::index::index::Index;
use crate::core::refs::Refs;
use crate::core::editor::Editor;
use crate::errors::error::Error;

pub const COMMIT_NOTES: &str = "Please enter the commit message for your changes. Lines starting with
'#' will be ignored, and an empty message aborts the commit.";

pub const MERGE_NOTES: &str = "It looks like you may be committing a merge.
If this is not correct, please remove the file
\t.ash/MERGE_HEAD
and try again.";

pub struct CommitWriter<'a> {
    root_path: &'a Path,
    git_path: PathBuf,
    pub database: &'a mut Database,
    pub index: &'a mut Index,
    pub refs: &'a Refs,
}

impl<'a> CommitWriter<'a> {
    pub fn new(
        root_path: &'a Path,
        git_path: PathBuf,
        database: &'a mut Database,
        index: &'a mut Index,
        refs: &'a Refs,
    ) -> Self {
        Self {
            root_path,
            git_path,
            database,
            index,
            refs,
        }
    }

    pub fn read_message(&self, message: Option<&str>, file: Option<&Path>) -> Result<String, Error> {
        let message = if let Some(message) = message {
            format!("{}\n", message)
        } else if let Some(file) = file {
            read_to_string(file)
                .map_err(|e| Error::Generic(format!("Failed to read message file: {}", e)))?
        } else {
            String::new()
        };

        Ok(message)
    }

    pub fn write_commit(&self, parents: Vec<String>, message: &str, author: Option<Author>) -> Result<Commit, Error> {
        if message.trim().is_empty() {
            return Err(Error::Generic("Aborting commit due to empty commit message".to_string()));
        }

        let tree = self.write_tree()?;
        
        // Use provided author or create a new one
        let author = author.unwrap_or_else(|| self.current_author());
        
        // For now, use the same author info for both author and committer fields
        // In a more advanced implementation, committer could be different
        let committer = author.clone();
        
        // Get the parent from the first element of parents or None
        let parent = parents.first().cloned();
        
        let mut commit = Commit::new(
            parent, // Use the first parent or None
            tree.get_oid().map(|s| s.to_string()).unwrap_or_default(),
            author,
            message.to_string()
        );

        self.database.store(&mut commit)?;
        
        // Get the commit OID, making sure we handle the option correctly
        let oid = commit.get_oid().map(|s| s.to_string()).unwrap_or_default();
        self.refs.update_head(&oid)?;

        Ok(commit)
    }

    pub fn write_tree(&self) -> Result<Tree, Error> {
        let entries = self.index.entries.values().cloned().collect();
        let root = Tree::build(entries);
        
        // Store all tree objects in the database
        let mut root_tree = root;
        root_tree.traverse_and_store(self.database)?;
        
        Ok(root_tree)
    }

    pub fn current_author(&self) -> Author {
        // Try to get author name from environment variables
        let name = std::env::var("GIT_AUTHOR_NAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "Unknown".to_string());
            
        // Try to get author email from environment variables
        let email = std::env::var("GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|_| format!("{}@localhost", name));
            
        // Use current time
        Author {
            name,
            email,
            timestamp: Local::now().into(),
        }
    }

    pub fn print_commit(&self, commit: &Commit) -> Result<(), Error> {
        // Get current branch name or HEAD
        let reference = self.refs.read_ref("HEAD")?.unwrap_or_default();
        let info = if reference.is_empty() {
            String::from("detached HEAD")
        } else {
            // Try to extract branch name from ref
            let branch_name = reference.strip_prefix("refs/heads/")
                .unwrap_or(&reference);
            branch_name.to_string()
        };
        
        // Get short OID
        let oid = commit.get_oid().map(|s| s.to_string()).unwrap_or_default();
        let short_oid = if oid.len() >= 7 {
            &oid[0..7]
        } else {
            &oid
        };
        
        // Add (root-commit) if this is the first commit
        let mut info_str = String::new();
        if commit.get_parent().is_none() {
            info_str.push_str(&format!("{} (root-commit)", info));
        } else {
            info_str.push_str(&info);
        }
        
        // Add commit hash
        info_str.push_str(&format!(" {}", short_oid));
        
        // Print commit info
        let title_line = commit.get_message().lines().next().unwrap_or("");
        println!("[{}] {}", info_str, title_line);
        
        Ok(())
    }

    pub fn compose_message(&self, editor_cmd: Option<String>, initial_message: Option<&str>) -> Result<Option<String>, Error> {
        self.edit_file(self.commit_message_path(), editor_cmd, |editor| {
            if let Some(msg) = initial_message {
                editor.write(msg)?;
            }
            editor.write("")?;
            editor.note(COMMIT_NOTES)?;
            Ok(())
        })
    }

    pub fn compose_merge_message(&self, editor_cmd: Option<String>, initial_message: &str) -> Result<Option<String>, Error> {
        self.edit_file(self.commit_message_path(), editor_cmd, |editor| {
            editor.write(initial_message)?;
            editor.note(MERGE_NOTES)?;
            editor.write("")?;
            editor.note(COMMIT_NOTES)?;
            Ok(())
        })
    }

    pub fn edit_file<F>(&self, path: PathBuf, editor_cmd: Option<String>, f: F) -> Result<Option<String>, Error>
    where
        F: FnOnce(&mut Editor) -> Result<(), Error>,
    {
        Editor::edit(path, editor_cmd, f)
    }

    pub fn commit_message_path(&self) -> PathBuf {
        self.git_path.join("COMMIT_EDITMSG")
    }

    pub fn handle_conflicted_index(&self) -> Result<(), Error> {
        if !self.index.has_conflict() {
            return Ok(());
        }

        println!("error: Committing is not possible because you have unmerged files.");
        println!("Fix them up in the work tree, and then use 'ash add <file>' as appropriate to mark resolution and make a commit.");
        println!("fatal: Exiting because of an unresolved conflict.");

        Err(Error::Generic("Unresolved conflicts exist in the index".to_string()))
    }
} 