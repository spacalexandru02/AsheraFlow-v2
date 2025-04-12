use std::time::Instant;
use std::io::{self, Write};
use crate::errors::error::Error;
use crate::core::revision::Revision;
use crate::core::repository::repository::Repository;
use crate::core::color::Color;
use crate::core::refs::Reference;
use crate::core::database::commit::Commit;

pub struct CheckoutCommand;

impl CheckoutCommand {
    pub fn execute(target: &str) -> Result<(), Error> {
        let start_time = Instant::now();
        
        // Initialize repository
        let mut repo = Repository::new(".")?;
        
        // Read current reference information
        let current_ref = repo.refs.current_ref()?;
        let current_oid = match repo.refs.read_head()? {
            Some(oid) => Some(oid),
            None => None,
        };
        
        // Resolve the target revision to a commit ID
        let mut revision = Revision::new(&mut repo, target);
        let target_oid = match revision.resolve("commit") {
            Ok(oid) => oid,
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
        };
        
        // Create a tree diff between current and target commits
        let tree_diff = repo.tree_diff(current_oid.as_deref(), Some(&target_oid))?;
        
        // Load the index for update
        repo.index.load_for_update()?;
        
        // Create and apply migration
        let mut migration = repo.migration(tree_diff);
        
        match migration.apply_changes() {
            Ok(_) => {
                // Migration succeeded, write index updates
                repo.index.write_updates()?;
                
                // Update HEAD to point to the new target or branch
                repo.refs.set_head(target, &target_oid)?;
                
                // Get the new reference for output
                let new_ref = repo.refs.current_ref()?;
                
                // Print status information
                Self::print_checkout_status(&repo, &current_ref, &current_oid, &new_ref, target, &target_oid)?;
                
                let elapsed = start_time.elapsed();
                println!("Checkout completed in {:.2}s", elapsed.as_secs_f32());
                
                Ok(())
            },
            Err(_e) => {
                // Migration failed
                // Clone the errors first before releasing the lock to avoid borrow conflicts
                let errors = migration.errors.clone();
                
                // Release index lock
                repo.index.rollback()?;
                
                // Print all error messages without referencing migration
                for message in errors {
                    eprintln!("error: {}", message);
                }
                
                eprintln!("Aborting");
                
                Err(Error::Generic("Checkout failed due to conflicts".to_string()))
            }
        }
    }
    
    // Print checkout status based on previous and current state
    fn print_checkout_status(
        repo: &Repository,
        current_ref: &Reference,
        current_oid: &Option<String>,
        new_ref: &Reference,
        target: &str,
        target_oid: &str
    ) -> Result<(), Error> {
        let stderr = io::stderr();
        let mut stderr_handle = stderr.lock();
        
        // Print previous HEAD position if we had a detached HEAD
        if Self::is_detached_head(current_ref) && current_oid.is_some() && current_oid.as_ref().map(|s| s.as_str()) != Some(target_oid) {
            if let Some(oid) = current_oid {
                Self::print_head_position(&mut stderr_handle, "Previous HEAD position was", oid, repo)?;
            }
        }
        
        // Print detachment notice if HEAD was attached but is now detached
        if Self::is_detached_head(new_ref) && !Self::is_detached_head(current_ref) {
            writeln!(stderr_handle, "Note: checking out '{}'.", target)?;
            writeln!(stderr_handle, "")?;
            writeln!(stderr_handle, "You are in 'detached HEAD' state. You can look around, make experimental")?;
            writeln!(stderr_handle, "changes and commit them, and you can discard any commits you make in this")?;
            writeln!(stderr_handle, "state without impacting any branches by performing another checkout.")?;
            writeln!(stderr_handle, "")?;
            writeln!(stderr_handle, "If you want to create a new branch to retain commits you create, you may")?;
            writeln!(stderr_handle, "do so (now or later) by using the branch command. Example:")?;
            writeln!(stderr_handle, "")?;
            writeln!(stderr_handle, "  ash branch <new-branch-name>")?;
            writeln!(stderr_handle, "")?;
        }
        
        // Print new HEAD information
        if Self::is_detached_head(new_ref) {
            // If we're in detached HEAD state, print its position
            Self::print_head_position(&mut stderr_handle, "HEAD is now at", target_oid, repo)?;
        } else {
            // If HEAD is attached to a branch, print the branch name
            match new_ref {
                Reference::Symbolic(path) => {
                    let branch_name = repo.refs.short_name(path);
                    if new_ref == current_ref {
                        writeln!(stderr_handle, "Already on '{}'", branch_name)?;
                    } else {
                        writeln!(stderr_handle, "Switched to branch '{}'", branch_name)?;
                    }
                },
                Reference::Direct(_) => {
                    // This shouldn't happen if we're not in detached HEAD state
                    writeln!(stderr_handle, "HEAD is at {}", &target_oid[0..8])?;
                }
            }
        }
        
        Ok(())
    }
    
    // Check if HEAD is detached (pointing directly to a commit)
    fn is_detached_head(reference: &Reference) -> bool {
        match reference {
            Reference::Direct(_) => true,
            Reference::Symbolic(path) => path == "HEAD"
        }
    }
    
    // Print HEAD position information
    fn print_head_position(
        writer: &mut impl Write,
        message: &str,
        oid: &str,
        repo: &Repository
    ) -> Result<(), Error> {
        // Use clone to avoid mutable borrow issues
        let commit_obj = repo.database.clone().load(oid)?;
        let short_oid = if oid.len() >= 8 { &oid[0..8] } else { oid };
        
        if let Some(commit) = commit_obj.as_any().downcast_ref::<Commit>() {
            let title = commit.title_line();
            writeln!(writer, "{} {} {}", message, Color::yellow(short_oid), title)?;
        } else {
            writeln!(writer, "{} {}", message, Color::yellow(short_oid))?;
        }
        
        Ok(())
    }
}