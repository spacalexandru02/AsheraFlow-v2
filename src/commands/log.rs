// src/commands/log.rs with all fixes applied
use std::time::Instant;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::errors::error::Error;
use crate::core::color::Color;
use crate::core::pager::Pager;
use crate::core::database::database::Database;
use crate::core::database::commit::Commit;
use crate::core::path_filter::PathFilter;
use crate::core::refs::{Refs, Reference};
use crate::core::revision::Revision;

pub struct LogCommand;

impl LogCommand {
    pub fn execute(revisions: &[String], options: &HashMap<String, String>) -> Result<(), Error> {
        let start_time = Instant::now();
        
        // Initialize repository components
        let root_path = std::path::Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verify .ash directory exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an ash repository (or any of the parent directories): .ash directory not found".into()));
        }
        
        let mut database = Database::new(git_path.join("objects"));
        let refs = Refs::new(&git_path);
        
        // Parse options
        let abbrev = options.get("abbrev").map_or(false, |v| v == "true");
        let format_default = "medium".to_string();
        let format = options.get("format").unwrap_or(&format_default);
        let patch = options.get("patch").map_or(false, |v| v == "true");
        let decorate_default = "auto".to_string();
        let decorate = options.get("decorate").unwrap_or(&decorate_default);
        
        // Initialize pager for output
        let mut pager = Pager::new();
        pager.start()?;
        
        // Determine the starting commit - Use HEAD if no revision is specified
        let head_oid = if revisions.is_empty() {
            refs.read_head()?.ok_or_else(|| Error::Generic("No HEAD commit found. Repository may be empty.".to_string()))?
        } else {
            // Resolve the requested revision to a commit ID
            let mut repo = crate::core::repository::repository::Repository::new(".")?;
            let mut revision = Revision::new(&mut repo, &revisions[0]);
            revision.resolve("commit")?
        };
        
        // Check for path filtering
        let mut path_filter = PathFilter::new();
        let mut path_args = Vec::new();
        
        for arg in revisions {
            let path = PathBuf::from(arg);
            if path.exists() {
                path_args.push(path);
            }
        }
        
        if !path_args.is_empty() {
            path_filter = PathFilter::build(&path_args);
        }
        
        // Build reverse ref map for decoration if needed
        let reverse_refs = if decorate != "no" {
            build_reverse_refs(&refs)?
        } else {
            HashMap::new()
        };
        
        // Get current ref for decoration
        let current_ref = if decorate != "no" {
            refs.current_ref()?
        } else {
            Reference::Direct(String::new())
        };
        
        // Iterate through history beginning with the start commit
        let mut oid = head_oid;
        let mut first = true;
        
        while !oid.is_empty() {
            let commit_obj = database.load(&oid)?;
            let commit = match commit_obj.as_any().downcast_ref::<Commit>() {
                Some(c) => c,
                None => return Err(Error::Generic(format!("Object {} is not a commit", oid))),
            };
            
            // Check if commit affects any of the filtered paths
            let commit_affects_paths = if !path_args.is_empty() {
                // Get parent commit
                let parent_oid = commit.get_parent();
                
                // Get diff between this commit and parent
                let diff = database.tree_diff(
                    parent_oid.as_deref().map(|s| s.as_str()), 
                    Some(&oid), 
                    &path_filter
                )?;
                
                // If diff is empty, this commit doesn't affect any of the paths
                !diff.is_empty()
            } else {
                // No path filtering, show all commits
                true
            };
            
            // Only show commit if it affects the filtered paths
            if commit_affects_paths {
                // Add a blank line between commits except before the first one
                if !first && format != "oneline" {
                    pager.write("\n")?;
                }
                first = false;
                
                // Display the commit based on format
                match format.as_str() {
                    "oneline" => {
                        show_commit_oneline(&mut pager, commit, abbrev, decorate, &reverse_refs, &current_ref)?;
                    },
                    _ => { // medium (default) format
                        show_commit_medium(&mut pager, commit, abbrev, decorate, &reverse_refs, &current_ref)?;
                    }
                }
                
                // Show patch if requested
                if patch {
                    if format != "oneline" {
                        pager.write("\n")?;
                    }
                    
                    // Get diff with possible path filtering
                    let parent_oid = commit.get_parent();
                    show_patch(
                        &mut pager, 
                        &mut database, 
                        parent_oid.as_deref().map(|s| s.as_str()), 
                        &oid, 
                        &path_filter
                    )?;
                }
            }
            
            // Move to parent commit
            if let Some(parent) = commit.get_parent() {
                oid = parent.clone();
            } else {
                break;
            }
            
            // Check if the pager was closed by the user
            if !pager.is_enabled() {
                break;
            }
        }
        
        // Display timing info
        if pager.is_enabled() {
            let elapsed = start_time.elapsed();
            pager.write(&format!("\n{}\n", Color::cyan(&format!("Log completed in {:.2}s", elapsed.as_secs_f32()))))?;
        }
        
        // Close the pager
        pager.close()?;
        
        Ok(())
    }
}

// Helper function to build a map from commit OIDs to the refs that point to them
fn build_reverse_refs(refs: &Refs) -> Result<HashMap<String, Vec<Reference>>, Error> {
    let mut reverse_refs = HashMap::new();
    
    // Get current HEAD reference
    if let Ok(Some(head_oid)) = refs.read_head() {
        let head_ref = Reference::Symbolic("HEAD".to_string());
        reverse_refs.entry(head_oid).or_insert_with(Vec::new).push(head_ref);
    }
    
    // Get all branch references
    let branches = refs.list_branches()?;
    for branch_ref in branches {
        if let Reference::Symbolic(path) = &branch_ref {
            if let Ok(Some(oid)) = refs.read_ref(path) {
                reverse_refs.entry(oid).or_insert_with(Vec::new).push(branch_ref.clone());
            }
        }
    }
    
    Ok(reverse_refs)
}

// Display a commit in the medium format (default)
fn show_commit_medium(
    pager: &mut Pager,
    commit: &Commit,
    abbrev: bool,
    decorate: &str,
    reverse_refs: &HashMap<String, Vec<Reference>>,
    current_ref: &Reference
) -> Result<(), Error> {
    // Format the commit ID
    let oid = if abbrev {
        commit.get_oid().map_or("".to_string(), |oid| {
            if oid.len() > 7 { oid[0..7].to_string() } else { oid.clone() }
        })
    } else {
        // Fix: Use cloned().unwrap_or_default() instead of unwrap_or_default().clone()
        commit.get_oid().cloned().unwrap_or_default()
    };
    
    // Add decoration if needed
    let decoration = if decorate != "no" {
        format_decoration(commit, reverse_refs, current_ref, decorate)
    } else {
        String::new()
    };
    
    // Display commit header
    pager.write(&format!("{} {}{}\n", Color::yellow("commit"), oid, decoration))?;
    
    // Display author information
    if let Some(author) = commit.get_author() {
        pager.write(&format!("Author: {} <{}>\n", author.name, author.email))?;
        pager.write(&format!("Date:   {}\n", author.short_date()))?;
    }
    
    // Display commit message
    pager.write("\n")?;
    for line in commit.get_message().lines() {
        pager.write(&format!("    {}\n", line))?;
    }
    
    Ok(())
}

// Display a commit in the oneline format
fn show_commit_oneline(
    pager: &mut Pager,
    commit: &Commit,
    abbrev: bool,
    decorate: &str,
    reverse_refs: &HashMap<String, Vec<Reference>>,
    current_ref: &Reference
) -> Result<(), Error> {
    // Format the commit ID
    let oid = if abbrev {
        commit.get_oid().map_or("".to_string(), |oid| {
            if oid.len() > 7 { oid[0..7].to_string() } else { oid.clone() }
        })
    } else {
        // Fix: Use cloned().unwrap_or_default() instead of unwrap_or_default().clone()
        commit.get_oid().cloned().unwrap_or_default()
    };
    
    // Add decoration if needed
    let decoration = if decorate != "no" {
        format_decoration(commit, reverse_refs, current_ref, decorate)
    } else {
        String::new()
    };
    
    // Get the first line of the commit message
    let title = commit.title_line();
    
    // Display the single line - Fix: use &oid for Color::yellow
    pager.write(&format!("{} {}{} {}\n", Color::yellow(&oid), decoration, "", title))?;
    
    Ok(())
}

// Format the decoration (refs) for a commit
fn format_decoration(
    commit: &Commit,
    reverse_refs: &HashMap<String, Vec<Reference>>,
    current_ref: &Reference,
    decorate: &str
) -> String {
    // If no refs point to this commit, return empty string
    if let Some(oid) = commit.get_oid() {
        if let Some(refs) = reverse_refs.get(oid) {
            if refs.is_empty() {
                return String::new();
            }
            
            // Format each ref name
            let mut ref_names = Vec::new();
            let mut has_head = false;
            
            for reference in refs {
                match reference {
                    Reference::Symbolic(path) => {
                        // Handle HEAD specially
                        if path == "HEAD" {
                            has_head = true;
                            continue;
                        }
                        
                        // Format branch name
                        let name = if decorate == "full" {
                            path.clone()
                        } else {
                            // Extract short name for branches
                            if path.starts_with("refs/heads/") {
                                path.strip_prefix("refs/heads/").unwrap_or(path).to_string()
                            } else {
                                path.clone()
                            }
                        };
                        
                        // Check if this is the current branch
                        if current_ref == reference {
                            if has_head {
                                ref_names.push(format!("{} -> {}", 
                                    Color::cyan("HEAD"), 
                                    Color::green(&name)));
                            } else {
                                ref_names.push(Color::green(&name));
                            }
                        } else {
                            ref_names.push(Color::green(&name));
                        }
                    },
                    Reference::Direct(direct_oid) => {
                        // Only include direct refs in full decoration mode
                        if decorate == "full" {
                            ref_names.push(direct_oid.clone());
                        }
                    }
                }
            }
            
            // If HEAD points to this commit but we have no branch to annotate
            if has_head && ref_names.is_empty() {
                ref_names.push(Color::cyan("HEAD"));
            }
            
            // Format the final decoration
            if !ref_names.is_empty() {
                return format!(" ({})", ref_names.join(", "));
            }
        }
    }
    
    String::new()
}

// Display the diff for a commit
fn show_patch(
    pager: &mut Pager,
    database: &mut Database,
    parent_oid: Option<&str>,
    commit_oid: &str,
    path_filter: &PathFilter
) -> Result<(), Error> {
    // Generate tree diff between parent and this commit with path filtering
    let diff = database.tree_diff(parent_oid, Some(commit_oid), path_filter)?;
    
    // If there are no changes, return early
    if diff.is_empty() {
        return Ok(());
    }
    
    // Sort paths for consistent output
    let mut paths: Vec<&PathBuf> = diff.keys().collect();
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    
    // Display each changed file
    for path in paths {
        let (old_entry, new_entry) = &diff[path];
        
        // Format file paths
        let path_str = path.to_string_lossy();
        let file_header = format!("diff --ash a/{} b/{}", path_str, path_str);
        pager.write(&format!("{}\n", Color::cyan(&file_header)))?;
        
        // Format mode changes if they differ
        if let (Some(old), Some(new)) = (old_entry, new_entry) {
            let old_mode = old.get_mode();
            let new_mode = new.get_mode();
            
            if old_mode != new_mode {
                let mode_line = format!("old mode {}, new mode {}", old_mode, new_mode);
                pager.write(&format!("{}\n", mode_line))?;
            }
            
            // Format index line with OIDs
            let old_oid = if old.get_oid().len() >= 7 { &old.get_oid()[0..7] } else { old.get_oid() };
            let new_oid = if new.get_oid().len() >= 7 { &new.get_oid()[0..7] } else { new.get_oid() };
            
            pager.write(&format!("index {}..{} {}\n", old_oid, new_oid, new_mode))?;
            
            // Display file headers
            pager.write(&format!("--- a/{}\n", path_str))?;
            pager.write(&format!("+++ b/{}\n", path_str))?;
            
            // Load and compare file contents
            let old_obj = database.load(old.get_oid())?;
            let new_obj = database.load(new.get_oid())?;
            
            let old_content = old_obj.to_bytes();
            let new_content = new_obj.to_bytes();
            
            // Check if files are binary
            if is_binary_content(&old_content) || is_binary_content(&new_content) {
                pager.write(&format!("{}\n", Color::yellow(&format!("Binary files a/{} and b/{} differ", path_str, path_str))))?;
                continue;
            }
            
            // Convert content to strings and show diff
            let old_text = String::from_utf8_lossy(&old_content);
            let new_text = String::from_utf8_lossy(&new_content);
            
            display_diff(pager, &old_text, &new_text)?;
        } else if let Some(old) = old_entry {
            // File was deleted
            let old_oid = if old.get_oid().len() >= 7 { &old.get_oid()[0..7] } else { old.get_oid() };
            
            pager.write(&format!("index {}..0000000\n", old_oid))?;
            pager.write(&format!("--- a/{}\n", path_str))?;
            pager.write(&format!("+++ /dev/null\n"))?;
            
            // Load and check file content
            let old_obj = database.load(old.get_oid())?;
            let old_content = old_obj.to_bytes();
            
            // Check if file is binary
            if is_binary_content(&old_content) {
                pager.write(&format!("{}\n", Color::yellow(&format!("Binary file a/{} has been deleted", path_str))))?;
                continue;
            }
            
            // Display deleted content
            let old_text = String::from_utf8_lossy(&old_content);
            let line_count = old_text.lines().count();
            
            pager.write(&format!("{}\n", Color::cyan(&format!("@@ -1,{} +0,0 @@", line_count))))?;
            
            for line in old_text.lines() {
                pager.write(&format!("{}\n", Color::red(&format!("-{}", line))))?;
            }
        } else if let Some(new) = new_entry {
            // File was added
            let new_oid = if new.get_oid().len() >= 7 { &new.get_oid()[0..7] } else { new.get_oid() };
            
            pager.write(&format!("index 0000000..{} {}\n", new_oid, new.get_mode()))?;
            pager.write(&format!("--- /dev/null\n"))?;
            pager.write(&format!("+++ b/{}\n", path_str))?;
            
            // Load and check file content
            let new_obj = database.load(new.get_oid())?;
            let new_content = new_obj.to_bytes();
            
            // Check if file is binary
            if is_binary_content(&new_content) {
                pager.write(&format!("{}\n", Color::yellow(&format!("Binary file b/{} has been created", path_str))))?;
                continue;
            }
            
            // Display added content
            let new_text = String::from_utf8_lossy(&new_content);
            let line_count = new_text.lines().count();
            
            pager.write(&format!("{}\n", Color::cyan(&format!("@@ -0,0 +1,{} @@", line_count))))?;
            
            for line in new_text.lines() {
                pager.write(&format!("{}\n", Color::green(&format!("+{}", line))))?;
            }
        }
    }
    
    Ok(())
}

// Display a diff between two files
fn display_diff(pager: &mut Pager, old_text: &str, new_text: &str) -> Result<(), Error> {
    // Split text into lines
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    
    // Simple implementation - in production would use Myers diff
    let hunk_header = format!("@@ -1,{} +1,{} @@", old_lines.len(), new_lines.len());
    pager.write(&format!("{}\n", Color::cyan(&hunk_header)))?;
    
    // Show all lines with appropriate marking
    if old_lines == new_lines {
        // Content is identical - show as context
        for line in &old_lines {
            pager.write(&format!(" {}\n", line))?;
        }
    } else {
        // Simple line-by-line diff
        for line in &old_lines {
            pager.write(&format!("{}\n", Color::red(&format!("-{}", line))))?;
        }
        
        for line in &new_lines {
            pager.write(&format!("{}\n", Color::green(&format!("+{}", line))))?;
        }
    }
    
    Ok(())
}

// Check if content appears to be binary
fn is_binary_content(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }
    
    // If it contains null bytes, it's probably binary
    if content.contains(&0) {
        return true;
    }
    
    // Check a sample for high proportion of non-printable characters
    let sample_size = content.len().min(8192);
    let sample = &content[0..sample_size];
    
    let non_text_count = sample.iter()
        .filter(|&&b| !b.is_ascii_graphic() && !b.is_ascii_whitespace())
        .count();
    
    (non_text_count as f64 / sample_size as f64) > 0.3
}