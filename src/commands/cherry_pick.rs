use std::collections::HashMap;
use std::path::Path;

use crate::core::database::commit::Commit;
use crate::core::database::database::Database;
use crate::core::refs::Refs;
use crate::errors::error::Error;
use crate::core::index::index::Index;
use crate::core::repository::sequencer::{Action, Sequencer};
use crate::core::revlist::RevList;
use crate::commands::commit_writer::CommitWriter;
use crate::core::revision::Revision;
use crate::core::repository::repository::Repository;

// Constants
const CONFLICT_NOTES: &str = "\
after resolving the conflicts, mark the corrected paths
with 'ash add <paths>' or 'ash rm <paths>'
and commit the result with 'ash commit'";

pub struct CherryPickCommand;

impl CherryPickCommand {
    pub fn execute(
        args: &[String],
        continue_op: bool,
        abort: bool,
        quit: bool,
        mainline: Option<u32>,
    ) -> Result<(), Error> {
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        let repo_path = git_path.clone();
        let db_path = git_path.join("objects");
        let index_path = git_path.join("index");

        // Verify repository exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an AsheraFlow repository: .ash directory not found".into()));
        }

        // Initialize repository
        let mut repo = Repository::new(".")?;
        
        // Create cherry-pick options map
        let mut options = HashMap::new();
        if let Some(mainline) = mainline {
            options.insert(String::from("mainline"), mainline.to_string());
        }

        // Initialize sequencer
        let mut sequencer = Sequencer::new(repo_path.clone());

        if continue_op {
            println!("Continuing cherry-pick operation...");
            sequencer.load()?;
            if sequencer.next_command().is_some() {
                println!("Continuing with the next commit");
                sequencer.drop_command()?;
            } else {
                println!("No commits left to cherry-pick");
            }
            return Ok(());
        } else if abort {
            println!("Aborting cherry-pick operation...");
            if let Err(e) = sequencer.abort() {
                println!("Warning during abort: {}", e);
            }
            return Ok(());
        } else if quit {
            println!("Quitting cherry-pick operation without aborting...");
            sequencer.quit()?;
            return Ok(());
        } else {
            println!("Starting cherry-pick operation for {} commits...", args.len());
            sequencer.start(&options)?;
            
            // Resolve each commit hash separately using Revision
            let mut resolved_oids = Vec::new();
            for arg in args {
                let mut revision = Revision::new(&mut repo, arg);
                match revision.resolve("commit") {
                    Ok(oid) => {
                        resolved_oids.push(oid);
                    },
                    Err(e) => {
                        // Handle invalid revision
                        for err in revision.errors {
                            eprintln!("error: {}", err.message);
                            for hint in &err.hint {
                                eprintln!("hint: {}", hint);
                            }
                        }
                        return Err(e);
                    }
                }
            }
            
            // Get the commits to cherry-pick using resolved OIDs
            let mut commits = Vec::new();
            for oid in resolved_oids {
                let commit_obj = repo.database.load(&oid)?;
                if let Some(commit) = commit_obj.as_any().downcast_ref::<Commit>() {
                    commits.push(commit.clone());
                } else {
                    return Err(Error::Generic(format!("Object {} is not a commit", oid)));
                }
            }
            
            // Add commits to the sequencer
            for commit in commits.iter().rev() {
                sequencer.add_pick(commit.clone());
            }
            
            println!("Added {} commits to cherry-pick", commits.len());
        }
        
        // Process the first commit
        if let Some((action, commit)) = sequencer.next_command() {
            // Initialize commit writer
            let mut commit_writer = CommitWriter::new(
                root_path,
                repo_path,
                &mut repo.database,
                &mut repo.index,
                &repo.refs
            );
            
            match action {
                Action::Pick => {
                    let commit_oid = commit.get_oid().map_or_else(String::new, |s| s.clone());
                    println!("Cherry-picking commit: {}", commit_oid);
                    
                    // Get original author and message
                    let author = match commit.get_author() {
                        Some(a) => a.clone(),
                        None => commit_writer.current_author()
                    };
                    let message = commit.get_message().to_string();
                    
                    // Get the current HEAD as parent
                    let head_ref = repo.refs.read_head()?.unwrap_or_default();
                    
                    // Use CommitWriter to handle the commit creation
                    let parents = vec![head_ref];
                    let new_commit = commit_writer.write_commit(parents, &message, Some(author))?;
                    
                    // Print commit information
                    commit_writer.print_commit(&new_commit)?;
                    
                    // Remove the cherry-pick command from the sequencer
                    sequencer.drop_command()?;
                    println!("Successfully cherry-picked commit");
                },
                Action::Revert => {
                    return Err(Error::Generic("Revert action not supported in cherry-pick".into()));
                }
            }
        }
        
        Ok(())
    }
} 