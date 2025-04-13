use crate::cli::args::{CliArgs, Command};
use crate::errors::error::Error;

pub struct CliParser;

impl CliParser {
    pub fn parse(args: Vec<String>) -> Result<CliArgs, Error> {
        if args.len() < 2 {
            // Return help message if no command is provided
             return Err(Error::Generic(format!("{}\n\n{}",
                 "Usage: ash <command> [options]",
                 Self::format_help() // Include help format on basic usage error
             )));
        }

        let command = args[1].to_lowercase();
        let cli_args = match command.as_str() {
            "init" => CliArgs {
                command: Command::Init {
                    path: args.get(2).map(|s| s.to_owned()).unwrap_or(".".to_string()),
                },
            },
            "commit" => {
                let mut message = None; // Use Option for message initially
                let mut i = 2;
                while i < args.len() {
                    if (args[i] == "--message" || args[i] == "-m") && i + 1 < args.len() {
                        message = Some(args[i + 1].to_owned());
                        i += 2; // Skip both flag and value
                    } else {
                        // Handle potential unknown flags or arguments here if needed
                        i += 1;
                    }
                }

                if message.is_none() {
                     // Try reading from standard input or editor if no -m is provided (like git)
                     // For now, require the message flag
                    return Err(Error::Generic("Commit message is required. Use --message <msg> or -m <msg>".to_string()));
                }

                CliArgs {
                    command: Command::Commit {
                        message: message.unwrap(), // Unwrap is safe here due to check above
                    },
                }
            },
            "add" => {
                if args.len() < 3 {
                    return Err(Error::Generic("File path(s) are required for add command".to_string()));
                }
                CliArgs {
                    command: Command::Add {
                        paths: args[2..].to_vec(),
                    },
                }
            },
            "status" => {
                // Check for --porcelain flag
                let porcelain = args.iter().skip(2).any(|arg| arg == "--porcelain");

                // Check for --color option
                let color = args.iter().skip(2).enumerate().find_map(|(i, arg)| {
                    // Correct index check for color value
                    if arg == "--color" && i + 2 < args.len() { // Look at index i+2 relative to args start
                        Some(args[i + 2 + 1].clone()) // Argument is at i+2, value at i+3 (relative to args start)
                    } else if arg.starts_with("--color=") {
                        Some(arg.splitn(2, '=').nth(1).unwrap_or("auto").to_string())
                    } else {
                        None
                    }
                }).unwrap_or_else(|| "auto".to_string()); // Default to auto

                CliArgs {
                    command: Command::Status {
                        porcelain,
                        color,
                    },
                }
            },
            "diff" => {
                // Parse diff command arguments
                let mut paths = Vec::new();
                let mut cached = false;

                // Check for --cached or --staged flag
                for arg in args.iter().skip(2) {
                    if arg == "--cached" || arg == "--staged" {
                        cached = true;
                    } else if !arg.starts_with('-') { // Assume non-flag arguments are paths
                        paths.push(arg.clone());
                    } else {
                         // Handle other potential flags or return error for unknown flags
                         // return Err(Error::Generic(format!("Unknown option for diff: {}", arg)));
                    }
                }

                CliArgs {
                    command: Command::Diff {
                        paths,
                        cached,
                    },
                }
            },
            "branch" => {
                // Parse branch options
                let mut name = String::new();
                let mut start_point = None;
                let mut verbose = false;
                let mut delete = false;
                let mut force = false;

                // Process all arguments for options
                let mut i = 2;
                while i < args.len() {
                    let arg = &args[i];
                    match arg.as_str() {
                        "-v" | "--verbose" => {
                            verbose = true;
                        },
                        "-d" | "--delete" => {
                            delete = true;
                        },
                        "-f" | "--force" => {
                            force = true;
                        },
                        "-D" => {
                            delete = true;
                            force = true;
                        },
                        // Check for other potential flags if needed
                        a if a.starts_with('-') => {
                            // Allow flags to appear anywhere relative to positional args
                            // Just consume the flag
                        },
                        _ => {
                            // Treat non-flag arguments as positional: name then start_point
                            if name.is_empty() {
                                name = arg.clone();
                            } else if start_point.is_none() {
                                start_point = Some(arg.clone());
                            } else {
                                 // Too many positional arguments
                                 return Err(Error::Generic(format!("Unexpected argument for branch: {}", arg)));
                            }
                        }
                    }
                    i += 1; // Increment index for every argument processed
                }

                // If name is empty, it implies listing branches (handled by BranchCommand)
                // If delete is true, name must be provided
                if delete && name.is_empty() {
                     return Err(Error::Generic("Branch name required for delete operation".to_string()));
                }


                CliArgs {
                    command: Command::Branch {
                        name, // Can be empty for listing
                        start_point,
                        verbose,
                        delete,
                        force
                    },
                }
            },
            "checkout" => {
                if args.len() < 3 {
                    return Err(Error::Generic("No checkout target specified (branch, commit, or path)".to_string()));
                }
                 // Allow multiple targets for file checkout? Git's behavior is complex here.
                 // For now, assume one target (branch or commit).
                 // Handle `checkout -- <paths...>` separately if needed.
                let target = args[2].clone();

                CliArgs {
                    command: Command::Checkout {
                        target,
                    },
                }
            },
            "log" => {
                // Parse log command options
                let mut revisions = Vec::new();
                let mut abbrev = false; // Default to false like git
                let mut format = "medium".to_string();
                let mut patch = false;
                let mut decorate = "auto".to_string();

                // Process arguments
                let mut i = 2;
                while i < args.len() {
                    let arg = &args[i];
                    match arg.as_str() {
                        "--abbrev-commit" => {
                            abbrev = true;
                        },
                        "--no-abbrev-commit" => {
                            abbrev = false;
                        },
                        "--pretty" | "--format" => {
                            if i + 1 < args.len() {
                                format = args[i + 1].clone();
                                i += 1; // Increment extra for the value
                            } else {
                                 return Err(Error::Generic(format!("Option '{}' requires a value", arg)));
                            }
                        },
                        a if a.starts_with("--pretty=") || a.starts_with("--format=") => {
                             let parts: Vec<&str> = a.splitn(2, '=').collect();
                             if parts.len() == 2 {
                                format = parts[1].to_string();
                             } else {
                                 return Err(Error::Generic(format!("Invalid format for option '{}'", arg)));
                             }
                        },
                        "--oneline" => {
                            format = "oneline".to_string();
                            abbrev = true; // oneline implies abbrev
                        },
                        "-p" | "-u" | "--patch" => {
                            patch = true;
                        },
                        "-s" | "--no-patch" => {
                            patch = false;
                        },
                        "--decorate" => {
                            // Allow setting decorate without a value, default to short/auto later
                             decorate = "auto".to_string();
                        },
                        a if a.starts_with("--decorate=") => {
                             let parts: Vec<&str> = a.splitn(2, '=').collect();
                             if parts.len() == 2 {
                                decorate = parts[1].to_string();
                             } else {
                                 return Err(Error::Generic(format!("Invalid format for option '{}'", arg)));
                             }
                        },
                        "--no-decorate" => {
                            decorate = "no".to_string();
                        },
                        a if a.starts_with('-') => {
                            // Unknown flag
                             return Err(Error::Generic(format!("Unknown option for log: {}", a)));
                        },
                        _ => {
                            // This is a revision specifier
                            revisions.push(arg.clone());
                        }
                    }
                     i += 1; // Increment for the current argument
                }

                CliArgs {
                    command: Command::Log {
                        revisions,
                        abbrev,
                        format,
                        patch,
                        decorate,
                    },
                }
            },
            "rm" => {
                // Parse rm command options
                let mut files = Vec::new();
                let mut cached = false;
                let mut force = false;
                let mut recursive = false;
                
                // Process arguments
                let mut i = 2;
                while i < args.len() {
                    let arg = &args[i];
                    match arg.as_str() {
                        "--cached" => {
                            cached = true;
                        },
                        "-f" | "--force" => {
                            force = true;
                        },
                        "-r" | "--recursive" => {
                            recursive = true;
                        },
                        a if a.starts_with('-') => {
                            // Unknown flag
                            return Err(Error::Generic(format!("Unknown option for rm: {}", a)));
                        },
                        _ => {
                            // This is a file path
                            files.push(arg.clone());
                        }
                    }
                    i += 1;
                }
                
                if files.is_empty() {
                    return Err(Error::Generic("No files specified for removal".to_string()));
                }
                
                CliArgs {
                    command: Command::Rm {
                        files,
                        cached,
                        force,
                        recursive,
                    },
                }
            },
            "merge" => {
                let mut branch = String::new();
                let mut message = None;
                let mut abort = false;
                let mut continue_merge = false;
                let mut tool = None; 

                let mut i = 2;
                while i < args.len() {
                    let arg = &args[i];
                    match arg.as_str() {
                        "--message" | "-m" => {
                            if i + 1 < args.len() {
                                message = Some(args[i + 1].clone());
                                i += 1; 
                            } else {
                                return Err(Error::Generic(format!("Option '{}' requires a value", arg)));
                            }
                        },
                        "--abort" => {
                            abort = true;
                        },
                        "--continue" => {
                            continue_merge = true;
                        },
                        "--tool" | "-t" => {  // Added tool handling
                            if i + 1 < args.len() {
                                tool = Some(args[i + 1].clone());
                                i += 1;
                            } else {
                                return Err(Error::Generic(format!("Option '{}' requires a value", arg)));
                            }
                        },
                        "--tool-only" => { 
                            tool = Some("default".to_string());
                        },
                        // Allow unknown flags for now or add error handling
                        _ if arg.starts_with('-') => {
                            return Err(Error::Generic(format!("Unknown option for merge: {}", arg)));
                        },
                        // Assume the first non-flag argument is the branch name
                        _ if branch.is_empty() => {
                            branch = arg.clone();
                        },
                        // Handle unexpected additional arguments
                        _ => {
                            return Err(Error::Generic(format!("Unexpected argument for merge: {}", arg)));
                        }
                    }
                    i += 1; // Increment index
                }

                // Special case: if --tool-only or --tool is provided without branch, it means
                // we want to just run the tool on existing conflicts
                let resolve_only = tool.is_some() && branch.is_empty() && !abort && !continue_merge;
                
                // Branch name is required unless --abort, --continue, or just running the tool
                if branch.is_empty() && !abort && !continue_merge && !resolve_only {
                    return Err(Error::Generic("No branch specified for merge and not using --abort, --continue, or --tool".to_string()));
                }
                
                // Cannot specify branch name with --abort or --continue
                if !branch.is_empty() && (abort || continue_merge) {
                    return Err(Error::Generic("Cannot specify branch name with --abort or --continue".to_string()));
                }

                CliArgs {
                    command: Command::Merge {
                        branch,
                        message,
                        abort,
                        continue_merge,
                        tool,
                    },
                }
            },
            _ => CliArgs {
                command: Command::Unknown { name: command },
            },
        };

        Ok(cli_args)
    }

    pub fn format_help() -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            "Usage: ash <command> [options]",
            "Commands:",
            "  init [path]                       Initialize a new repository",
            "  add <paths...>                    Add file contents to the index",
            "  commit -m <message>               Commit changes to the repository",
            "  status [--porcelain] [--color=...] Show the working tree status",
            "  diff [--cached] [paths...]        Show changes (HEAD vs index or index vs workspace)",
            "  branch [-v] [-d|-D] [<n> [<sp>]]  Manage branches (list, create, delete)",
            "  checkout <target>                 Switch branches or restore working tree files",
            "  log [--oneline] [--decorate=...]  Show commit logs",
            "  merge <branch> [-m <msg>]         Merge the specified branch into the current branch",
            "        --abort                     Abort the current merge resolution process",
            "        --continue                  Continue the merge after resolving conflicts",
            "        --tool=<tool>               Use specified tool to resolve merge conflicts",
            "        --tool-only                 Run merge tool to resolve conflicts without merging",
            "Common Options:",
            "  (Options specific to commands listed above)",
            "  --help                           Display this help message"
        )
    }
}