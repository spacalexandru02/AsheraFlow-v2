use std::collections::HashMap;
use std::env;
use std::process;
use cli::args::Command;
use cli::parser::CliParser;
use commands::checkout::CheckoutCommand;
use commands::commit::CommitCommand;
use commands::diff::DiffCommand;
use commands::init::InitCommand;
use commands::add::AddCommand;
use commands::log::LogCommand;
use commands::status::StatusCommand;
use commands::branch::BranchCommand;
// --- Adaugă import pentru MergeCommand și MergeToolCommand ---
use commands::merge::MergeCommand;
use commands::merge_tool::MergeToolCommand;
use commands::rm::RmCommand;
use commands::reset::ResetCommand;
use std::path::Path;
use crate::core::index::index::Index;
use crate::core::refs::Refs;
use crate::errors::error::Error;
use std::time::Instant;
use crate::core::repository::repository::Repository;

mod cli;
mod commands;
mod validators;
mod errors;
mod core;

// Definim constanta ORIG_HEAD local
const ORIG_HEAD: &str = "ORIG_HEAD";

fn main() {
    let args: Vec<String> = env::args().collect();

    match CliParser::parse(args) {
        Ok(cli_args) => {
            match cli_args.command {
                Command::Init { path } => handle_init_command(&path),
                Command::Commit { message, amend, reuse_message, edit } => 
                    handle_commit_command(&message),
                Command::Add { paths } => handle_add_command(&paths),
                Command::Status { porcelain, color } => handle_status_command(porcelain, &color),
                Command::Diff { paths, cached } => handle_diff_command(&paths, cached),
                Command::Branch { name, start_point, verbose, delete, force } => {
                    handle_branch_command(&name, start_point.as_deref(), verbose, delete, force)
                },
                Command::Checkout { target } => handle_checkout_command(&target),
                Command::Log { revisions, abbrev, format, patch, decorate } => {
                    handle_log_command(&revisions, abbrev, &format, patch, &decorate)
                },
                Command::Merge { branch, message, abort, continue_merge, tool } => {
                    if abort {
                        handle_merge_abort_command();
                    } else if continue_merge {
                        handle_merge_continue_command();
                    } else if tool.is_some() && branch.is_empty() {
                        handle_merge_tool_command(tool.as_deref());
                    } else {
                        handle_merge_command(&branch, message.as_deref());
                    }
                },
                Command::Rm { files, cached, force, recursive } => {
                    handle_rm_command(&files, cached, force, recursive)
                },
                Command::Reset { files, soft, mixed, hard, force, reuse_message } => {
                    handle_reset_command(&files, soft, mixed, hard, force, reuse_message.as_deref())
                },
                Command::Unknown { name } => {
                    println!("Unknown command: {}", name);
                    println!("{}", CliParser::format_help());
                    process::exit(1);
                }
            }
        },
        Err(e) => {
            if e.to_string().contains("Usage:") {
                // Handle the case where no command is given
                println!("{}", e);
            } else {
                println!("Error parsing command: {}", e);
            }
            process::exit(1);
        }
    }
}

fn handle_commit_command(message: &str) {
    match CommitCommand::execute(message) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn handle_init_command(path: &str) {
    match InitCommand::execute(path) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn handle_add_command(paths: &[String]) {
    match AddCommand::execute(paths) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn handle_status_command(porcelain: bool, color: &str) {
    // Set color mode environment variable
    std::env::set_var("ASH_COLOR", color);

    match StatusCommand::execute(porcelain) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn handle_diff_command(paths: &[String], cached: bool) {
    match DiffCommand::execute(paths, cached) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn handle_branch_command(name: &str, start_point: Option<&str>, verbose: bool, delete: bool, force: bool) {
    // Set environment variables to pass flag information
    if verbose {
        std::env::set_var("ASH_BRANCH_VERBOSE", "1");
    }
    if delete {
        std::env::set_var("ASH_BRANCH_DELETE", "1");
    }
    if force {
        std::env::set_var("ASH_BRANCH_FORCE", "1");
    }

    match BranchCommand::execute(name, start_point) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}


fn handle_log_command(revisions: &[String], abbrev: bool, format: &str, patch: bool, decorate: &str) {
    // Convert options to HashMap for easier handling
    let mut options = HashMap::new();
    options.insert("abbrev".to_string(), abbrev.to_string());
    options.insert("format".to_string(), format.to_string());
    options.insert("patch".to_string(), patch.to_string());
    options.insert("decorate".to_string(), decorate.to_string());

    match LogCommand::execute(revisions, &options) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn handle_checkout_command(target: &str) {
    match CheckoutCommand::execute(target) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

// Add function to handle merge_tool command
fn handle_merge_tool_command(tool: Option<&str>) {
    match MergeToolCommand::execute(tool) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

/// Handles merge continue operation
fn handle_merge_continue_command() {
    println!("Checking for unresolved conflicts...");
    
    // Initialize repository components
    let root_path = Path::new(".");
    let git_path = root_path.join(".ash");
    
    if !git_path.exists() {
        println!("Not an AsheraFlow repository: .ash directory not found");
        process::exit(1);
    }
    
    // Check if we can access the index
    let mut index = Index::new(git_path.join("index"));
    match index.load() {
        Ok(_) => {
            // Check if there are unresolved conflicts
            if index.has_conflict() {
                println!("There are still unresolved conflicts.");
                println!("Fix the conflicts first, then run 'ash merge --continue'");
                process::exit(1);
            }
        },
        Err(e) => {
            println!("Error loading index: {}", e);
            process::exit(1);
        }
    }
    
    // All conflicts are resolved, complete the merge with a commit
    println!("All conflicts resolved. Creating merge commit...");
    
    // Generate a default merge message
    let message = "Merge branch (conflicts resolved)";
    
    // In merge --continue we don't need to specify a branch, so use empty string
    match CommitCommand::execute(message) {
        Ok(_) => {
            println!("Merge completed successfully.");
            process::exit(0);
        },
        Err(e) => {
            println!("Error completing merge: {}", e);
            process::exit(1);
        }
    }
}

fn handle_rm_command(files: &[String], cached: bool, force: bool, recursive: bool) {
    match RmCommand::execute(files, cached, force, recursive) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn handle_reset_command(files: &[String], soft: bool, mixed: bool, hard: bool, force: bool, reuse_message: Option<&str>) {
    match ResetCommand::execute(files, soft, mixed, hard, force, reuse_message) {
        Ok(_) => process::exit(0),
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    }
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("{}", message); // Afișează eroarea pe stderr
    // Poți adăuga logica de afișare a mesajului de ajutor aici dacă dorești
    // if message.contains("Usage:") || ... {
    //     eprintln!("\n{}", CliParser::format_help());
    // }
    process::exit(1); // Ieșim cu cod de eroare (1)
}

// --- Păstrează funcția handle_merge_command originală ---
fn handle_merge_command(branch: &str, message: Option<&str>) {
    match MergeCommand::execute(branch, message) {
        Ok(_) => process::exit(0),
        Err(e) => {
            // Pentru erori specifice de merge, dorim să afișăm un mesaj mai clar
            if e.to_string().contains("Already up to date") {
                println!("Already up to date.");
                process::exit(0);
            } else if e.to_string().contains("fix conflicts") {
                // Dacă există conflicte, dorim să afișăm un mesaj de eroare mai clar
                println!("{}", e);
                println!("Conflicts detected. Fix conflicts and then run 'ash merge --continue'");
                process::exit(1);
            } else {
                exit_with_error(&format!("fatal: {}", e));
            }
        }
    }
}

// Funcție pentru a gestiona merge abort
fn handle_merge_abort_command() {
    // Inițializare repository
    let mut repo = match Repository::new(".") {
        Ok(r) => r,
        Err(e) => exit_with_error(&format!("fatal: {}", e)),
    };
    
    // Verificăm dacă există un merge în desfășurare
    let git_path = Path::new(".").join(".ash");
    let merge_head_path = git_path.join("MERGE_HEAD");
    if !merge_head_path.exists() {
        exit_with_error("fatal: There is no merge to abort");
    }
    
    // Ștergem fișierele specifice merge-ului
    let _ = std::fs::remove_file(merge_head_path);
    let _ = std::fs::remove_file(git_path.join("MERGE_MSG"));
    
    // Citim HEAD-ul original
    let orig_head_path = git_path.join(ORIG_HEAD);
    let orig_head = match std::fs::read_to_string(&orig_head_path) {
        Ok(content) => content.trim().to_string(),
        Err(e) => exit_with_error(&format!("fatal: Failed to read ORIG_HEAD: {}", e)),
    };
    
    // Folosim ResetCommand pentru a face un hard reset la starea originală
    match ResetCommand::execute(&[orig_head], false, false, true, true, None) {
        Ok(_) => {
            println!("Merge aborted");
            process::exit(0);
        },
        Err(e) => exit_with_error(&format!("fatal: Failed to reset to ORIG_HEAD: {}", e)),
    }
}