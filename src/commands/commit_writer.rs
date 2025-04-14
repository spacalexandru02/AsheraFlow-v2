use std::fs::read_to_string;
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::core::database::author::Author;
use crate::core::database::commit::Commit;
use crate::core::database::database::{Database, GitObject};
use crate::core::database::entry::DatabaseEntry;
use crate::core::database::tree::Tree;
use crate::core::index::index::Index;
use crate::core::refs::Refs;
use crate::core::editor::Editor;
use crate::core::repository::pending_commit::{PendingCommit, PendingCommitType};
use crate::errors::error::Error;

pub const COMMIT_NOTES: &str = "Please enter the commit message for your changes. Lines starting with
'#' will be ignored, and an empty message aborts the commit.";

pub const MERGE_NOTES: &str = "It looks like you may be committing a merge.
If this is not correct, please remove the file
\t.ash/MERGE_HEAD
and try again.";

pub const CHERRY_PICK_NOTES: &str = "It looks like you may be committing a cherry-pick.
If this is not correct, please remove the file
\t.ash/CHERRY_PICK_HEAD
and try again.";

pub const CONFLICT_MESSAGE: &str = "hint: Fix them up in the work tree, and then use 'ash add <file>'
hint: as appropriate to mark resolution and make a commit.
fatal: Exiting because of an unresolved conflict.";

pub struct CommitWriter<'a> {
    root_path: &'a Path,
    git_path: PathBuf,
    pub database: &'a mut Database,
    pub index: &'a mut Index,
    pub refs: &'a Refs,
    pub pending_commit: PendingCommit,
}

impl<'a> CommitWriter<'a> {
    pub fn new(
        root_path: &'a Path,
        git_path: PathBuf,
        database: &'a mut Database,
        index: &'a mut Index,
        refs: &'a Refs,
    ) -> Self {
        let pending_commit = PendingCommit::new(&git_path);
        
        Self {
            root_path,
            git_path,
            database,
            index,
            refs,
            pending_commit,
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

    pub fn write_commit(&mut self, parents: Vec<String>, message: &str, author: Option<Author>) -> Result<Commit, Error> {
        if message.trim().is_empty() {
            return Err(Error::Generic("Aborting commit due to empty commit message".to_string()));
        }

        let tree = self.write_tree()?;
        
        // Use provided author or create a new one
        let author = author.unwrap_or_else(|| self.current_author());
        
        // Use current author as committer 
        let committer = self.current_author();
        
        // Get the first parent or None
        let parent = parents.first().cloned();
        
        let mut commit = Commit::new_with_committer(
            parent,
            tree.get_oid().map(|s| s.to_string()).unwrap_or_default(),
            author,
            committer,
            message.to_string()
        );

        self.database.store(&mut commit)?;
        
        // Get the commit OID, making sure we handle the option correctly
        let oid = commit.get_oid().map(|s| s.to_string()).unwrap_or_default();
        self.refs.update_head(&oid)?;

        Ok(commit)
    }

    pub fn write_tree(&mut self) -> Result<Tree, Error> {
        // Create a collection of DatabaseEntry from index entries
        let entries: Vec<DatabaseEntry> = self.index.entries.values()
            .map(|entry| DatabaseEntry::new(
                entry.get_path().to_string(),
                entry.get_oid().to_string(),
                &entry.mode_octal()
            ))
            .collect();
        
        let mut root = Tree::build(entries.iter())?;
        
        // Store all tree objects in the database
        root.traverse(|tree| {
            self.database.store(tree)?;
            Ok(())
        })?;
        
        Ok(root)
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

    pub fn compose_message(&mut self, editor_cmd: Option<String>, initial_message: Option<&str>) -> Result<Option<String>, Error> {
        self.edit_file(self.commit_message_path(), editor_cmd, |editor| {
            if let Some(msg) = initial_message {
                editor.write(msg)?;
            }
            editor.write("")?;
            editor.note(COMMIT_NOTES)?;
            Ok(())
        })
    }

    pub fn compose_merge_message(&mut self, editor_cmd: Option<String>, initial_message: &str, notes: Option<&str>) -> Result<Option<String>, Error> {
        self.edit_file(self.commit_message_path(), editor_cmd, |editor| {
            editor.write(initial_message)?;
            
            if let Some(notes) = notes {
                editor.note(notes)?;
            }
            
            editor.write("")?;
            editor.note(COMMIT_NOTES)?;
            Ok(())
        })
    }

    pub fn edit_file<F>(&mut self, path: PathBuf, editor_cmd: Option<String>, f: F) -> Result<Option<String>, Error>
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
        println!("{}", CONFLICT_MESSAGE);

        Err(Error::Generic("Unresolved conflicts exist in the index".to_string()))
    }
    
    // New methods for amending commits and handling merger operations
    
    pub fn handle_amend(&mut self, editor_cmd: Option<String>) -> Result<(), Error> {
        let head_oid = self.refs.read_head()?
            .ok_or_else(|| Error::Generic("No commit to amend".to_string()))?;
            
        let old_commit_obj = self.database.load(&head_oid)?;
        let old_commit = old_commit_obj.as_any().downcast_ref::<Commit>()
            .ok_or_else(|| Error::Generic("Invalid commit object".to_string()))?;
            
        let tree = self.write_tree()?;
        let message = self.compose_message(editor_cmd, Some(old_commit.get_message()))?
            .ok_or_else(|| Error::Generic("Aborting commit due to empty commit message".to_string()))?;
            
        // Get the author from the old commit
        let author = old_commit.get_author()
            .ok_or_else(|| Error::Generic("No author in commit".to_string()))?
            .clone();
            
        // Use current author as committer
        let committer = self.current_author();
        
        // Create new commit with the same parent(s) as the old commit
        let parent = old_commit.get_parent().cloned();
        
        let mut new_commit = Commit::new_with_committer(
            parent,
            tree.get_oid().map(|s| s.to_string()).unwrap_or_default(),
            author,
            committer,
            message
        );
        
        self.database.store(&mut new_commit)?;
        
        // Update HEAD to point to the new commit
        let new_oid = new_commit.get_oid()
            .ok_or_else(|| Error::Generic("New commit has no OID".to_string()))?;
            
        self.refs.update_head(new_oid)?;
        
        self.print_commit(&new_commit)?;
        
        Ok(())
    }
    
    pub fn reused_message(&mut self, revision: &str) -> Result<Option<String>, Error> {
        // TODO: Implement revision parsing to get the commit
        // For now, just try to use the OID directly
        let commit_obj = self.database.load(revision)?;
        let commit = commit_obj.as_any().downcast_ref::<Commit>()
            .ok_or_else(|| Error::Generic("Invalid commit object".to_string()))?;
            
        Ok(Some(commit.get_message().to_string()))
    }
    
    pub fn resume_merge(&mut self, r#type: PendingCommitType, editor_cmd: Option<String>) -> Result<(), Error> {
        self.handle_conflicted_index()?;
        
        let notes = match r#type {
            PendingCommitType::Merge => Some(MERGE_NOTES),
            PendingCommitType::CherryPick => Some(CHERRY_PICK_NOTES),
            PendingCommitType::Revert => None,
        };
        
        match r#type {
            PendingCommitType::Merge => self.write_merge_commit(editor_cmd, notes)?,
            PendingCommitType::CherryPick => self.write_cherry_pick_commit(editor_cmd, notes)?,
            PendingCommitType::Revert => self.write_revert_commit(editor_cmd)?,
        }
        
        Ok(())
    }
    
    fn write_merge_commit(&mut self, editor_cmd: Option<String>, notes: Option<&str>) -> Result<(), Error> {
        let parents = vec![
            self.refs.read_head()?.unwrap_or_default(),
            self.pending_commit.merge_oid(PendingCommitType::Merge)?,
        ];
        
        let merge_message = self.pending_commit.merge_message()?;
        let message = self.compose_merge_message(editor_cmd, &merge_message, notes)?
            .ok_or_else(|| Error::Generic("Aborting merge commit due to empty message".to_string()))?;
            
        let commit = self.write_commit(parents, &message, None)?;
        self.print_commit(&commit)?;
        
        self.pending_commit.clear(PendingCommitType::Merge)?;
        
        Ok(())
    }
    
    pub fn write_cherry_pick_commit(&mut self, editor_cmd: Option<String>, notes: Option<&str>) -> Result<(), Error> {
        let parents = vec![
            self.refs.read_head()?.unwrap_or_default(),
        ];
        
        let pick_oid = self.pending_commit.merge_oid(PendingCommitType::CherryPick)?;
        let commit_obj = self.database.load(&pick_oid)?;
        let commit = commit_obj.as_any().downcast_ref::<Commit>()
            .ok_or_else(|| Error::Generic("Invalid commit object".to_string()))?;
            
        let author = commit.get_author()
            .ok_or_else(|| Error::Generic("No author in commit".to_string()))?
            .clone();
            
        let merge_message = self.pending_commit.merge_message()?;
        let message = self.compose_merge_message(editor_cmd, &merge_message, notes)?
            .ok_or_else(|| Error::Generic("Aborting cherry-pick commit due to empty message".to_string()))?;
            
        let commit = self.write_commit(parents, &message, Some(author))?;
        self.print_commit(&commit)?;
        
        self.pending_commit.clear(PendingCommitType::CherryPick)?;
        
        Ok(())
    }
    
    pub fn write_revert_commit(&mut self, editor_cmd: Option<String>) -> Result<(), Error> {
        let parents = vec![
            self.refs.read_head()?.unwrap_or_default(),
        ];
        
        let merge_message = self.pending_commit.merge_message()?;
        let message = self.compose_merge_message(editor_cmd, &merge_message, None)?
            .ok_or_else(|| Error::Generic("Aborting revert commit due to empty message".to_string()))?;
            
        let commit = self.write_commit(parents, &message, None)?;
        self.print_commit(&commit)?;
        
        self.pending_commit.clear(PendingCommitType::Revert)?;
        
        Ok(())
    }

    pub fn get_editor_command(&self) -> String {
        let editor = std::env::var("GIT_EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".to_string());
        
        editor
    }
} 