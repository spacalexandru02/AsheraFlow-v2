use std::time::Instant;
use crate::errors::error::Error;
use crate::core::repository::repository::Repository;
use crate::core::revision::Revision;
use crate::core::color::Color;
use crate::core::refs::Reference;
use crate::core::database::commit::Commit;

pub struct BranchCommand;

impl BranchCommand {
    pub fn execute(branch_name: &str, start_point: Option<&str>) -> Result<(), Error> {
        let start_time = Instant::now();
        
        // Get flags from environment variables (set in main.rs)
        let verbose = std::env::var("ASH_BRANCH_VERBOSE").unwrap_or_default() == "1";
        let delete = std::env::var("ASH_BRANCH_DELETE").unwrap_or_default() == "1";
        let force = std::env::var("ASH_BRANCH_FORCE").unwrap_or_default() == "1";
        
        // Handle no arguments - list branches
        if branch_name.is_empty() {
            return Self::list_branches(verbose);
        }
        
        // Handle delete branch
        if delete {
            return Self::delete_branch(branch_name, force);
        }
        
        // Default behavior: create a new branch
        Self::create_branch(branch_name, start_point, force)
    }
    
    // List all branches in the repository
    fn list_branches(verbose: bool) -> Result<(), Error> {
        let start_time = Instant::now();
        let mut repo = Repository::new(".")?;
        
        // Get current branch
        let current_ref = repo.refs.current_ref()?;
        
        // Get all branches
        let branches = repo.refs.list_branches()?;
        
        // Find the maximum branch name length for alignment (if verbose)
        let max_width = if verbose {
            branches.iter().map(|r| {
                match r {
                    Reference::Symbolic(path) => repo.refs.short_name(path).len(),
                    _ => 0,
                }
            }).max().unwrap_or(0)
        } else {
            0
        };
        
        // Sort branches by name
        let mut branch_names: Vec<(String, Reference)> = branches.iter().map(|r| {
            match r {
                Reference::Symbolic(path) => (repo.refs.short_name(path), r.clone()),
                _ => (String::new(), r.clone()),
            }
        }).collect();
        
        branch_names.sort_by(|a, b| a.0.cmp(&b.0));
        
        // Print each branch
        for (name, reference) in branch_names {
            let mut info = Self::format_branch(&reference, &current_ref, &repo);
            
            if verbose {
                let extended_info = Self::extended_branch_info(&reference, max_width, &name, &mut repo)?;
                info.push_str(&extended_info);
            }
            
            println!("{}", info);
        }
        
        let elapsed = start_time.elapsed();
        println!("\nBranch command completed in {:.2}s", elapsed.as_secs_f32());
        
        Ok(())
    }
    
    // Format a branch reference for display
    fn format_branch(reference: &Reference, current_ref: &Reference, repo: &Repository) -> String {
        match reference {
            Reference::Symbolic(path) => {
                let name = repo.refs.short_name(path);
                if reference == current_ref {
                    format!("* {}", Color::green(&name))
                } else {
                    format!("  {}", name)
                }
            },
            _ => String::new(), // Should not happen for branches
        }
    }
    
    // Get extended branch info for verbose output
    fn extended_branch_info(
        reference: &Reference, 
        max_width: usize, 
        name: &str,
        repo: &mut Repository
    ) -> Result<String, Error> {
        // Get the commit this branch points to
        let oid = match reference {
            Reference::Symbolic(path) => {
                // Read the reference directly
                repo.refs.read_ref(path)?
                    .ok_or_else(|| Error::Generic(format!("Could not resolve reference {}", path)))?
            },
            Reference::Direct(oid) => oid.clone(),
        };
        
        // Load the commit
        let commit_obj = repo.database.load(&oid)?;
        
        if let Some(commit) = commit_obj.as_any().downcast_ref::<Commit>() {
            // Get abbreviated commit ID
            let short_oid = if oid.len() >= 8 { &oid[0..8] } else { &oid };
            
            // Get the title line of the commit message
            let title = commit.title_line();
            
            // Add padding to align commit info
            let padding = " ".repeat(max_width.saturating_sub(name.len()));
            
            Ok(format!("{} {} {}", padding, Color::yellow(short_oid), title))
        } else {
            Ok(String::new())
        }
    }
    
    // Create a new branch
    fn create_branch(branch_name: &str, start_point: Option<&str>, force: bool) -> Result<(), Error> {
        let mut repo = Repository::new(".")?;
        
        // Determine the start point (commit OID)
        let start_oid = if let Some(revision_expr) = start_point {
            // Resolve the revision to a commit OID
            let mut revision = Revision::new(&mut repo, revision_expr);
            match revision.resolve("commit") {
                Ok(oid) => oid,
                Err(e) => {
                    // Print any additional error information collected during resolution
                    for err in revision.errors {
                        eprintln!("error: {}", err.message);
                        for hint in &err.hint {
                            eprintln!("hint: {}", hint);
                        }
                    }
                    return Err(e);
                }
            }
        } else {
            // Use HEAD as the default start point
            repo.refs.read_head()?.ok_or_else(|| {
                Error::Generic("Failed to resolve HEAD - repository may be empty".to_string())
            })?
        };
        
        // Create the branch
        match repo.refs.create_branch(branch_name, &start_oid) {
            Ok(_) => {
                println!("Created branch '{}' at {}", branch_name, &start_oid[0..8]);
                
                let start_time = Instant::now();
                let elapsed = start_time.elapsed();
                println!("Branch command completed in {:.2}s", elapsed.as_secs_f32());
                
                Ok(())
            },
            Err(e) => Err(e)
        }
    }
    
    // Delete a branch
    fn delete_branch(branch_name: &str, force: bool) -> Result<(), Error> {
        // Force is required for now since we don't have merge functionality
        if !force {
            eprintln!("error: The branch '{}' is not fully merged.", branch_name);
            eprintln!("If you are sure you want to delete it, run 'ash branch -D {}'", branch_name);
            return Err(Error::Generic("Branch not deleted".to_string()));
        }
        
        let mut repo = Repository::new(".")?;
        
        // Check if we're trying to delete the current branch
        let current_ref = repo.refs.current_ref()?;
        let is_current = match &current_ref {
            Reference::Symbolic(path) => {
                repo.refs.short_name(path) == branch_name
            },
            _ => false,
        };
        
        if is_current {
            return Err(Error::Generic(format!(
                "Cannot delete the branch '{}' which you are currently on.",
                branch_name
            )));
        }
        
        // Delete the branch
        match repo.refs.delete_branch(branch_name) {
            Ok(oid) => {
                // Get short OID for display
                let short_oid = if oid.len() >= 8 { &oid[0..8] } else { &oid };
                
                println!("Deleted branch {} (was {}).", branch_name, short_oid);
                
                Ok(())
            },
            Err(e) => Err(e),
        }
    }
}