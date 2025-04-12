// src/utils/pager.rs
use std::env;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use crate::errors::error::Error;

pub struct Pager {
    enabled: bool,
    command: String,
    process: Option<std::process::Child>,
    stdout: Option<std::process::ChildStdin>,
    early_exit: bool,  // Flag to track if user exited pager early
}

impl Pager {
    /// Creates a new pager, detecting the available command in the system
    pub fn new() -> Self {
        // Verify if we should use a pager at all (terminal output vs pipe)
        let force_pager = env::var("ASH_FORCE_PAGER").map(|v| v == "1").unwrap_or(false);
        
        // Skip pager if output is not to a terminal, unless forced
        let use_pager = force_pager || atty::is(atty::Stream::Stdout);
        
        if !use_pager {
            return Pager {
                enabled: false,
                command: "cat".to_string(),
                process: None,
                stdout: None,
                early_exit: false,
            };
        }
        
        // Check if there's an explicitly set pager command
        let command = if let Ok(pager) = env::var("ASH_PAGER") {
            pager
        } else if let Ok(pager) = env::var("PAGER") {
            pager
        } else {
            // Auto-detect available pager
            let candidates = ["less", "more", "cat", "pager"];
            for cmd in candidates {
                if Self::command_exists(cmd) {
                    if cmd == "less" {
                        return Pager {
                            enabled: true,
                            command: "less -FRX".to_string(), // -F: quit if one screen, -R: preserve ANSI colors, -X: don't clear screen
                            process: None,
                            stdout: None,
                            early_exit: false,
                        };
                    }
                    return Pager {
                        enabled: true,
                        command: cmd.to_string(),
                        process: None,
                        stdout: None,
                        early_exit: false,
                    };
                }
            }
            // If no pager is found, we'll use stdout directly
            "cat".to_string()
        };
        
        Pager {
            enabled: true,
            command,
            process: None,
            stdout: None,
            early_exit: false,
        }
    }
    
    /// Check if a command exists in the system
    fn command_exists(cmd: &str) -> bool {
        let check_cmd = if cfg!(target_os = "windows") {
            Command::new("where")
                .arg(cmd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        } else {
            Command::new("which")
                .arg(cmd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        };
        
        match check_cmd {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }
    
    /// Initialize the pager for use
    pub fn start(&mut self) -> Result<(), Error> {
        // If not enabled, do nothing
        if !self.enabled {
            return Ok(());
        }
        
        // Extract command and arguments
        let parts: Vec<&str> = self.command.split_whitespace().collect();
        if parts.is_empty() {
            return Err(Error::Generic("Invalid pager command".into()));
        }
        
        let mut cmd = Command::new(parts[0]);
        for arg in &parts[1..] {
            cmd.arg(arg);
        }
        
        // Configure stdin/stdout
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::inherit()) // Connect the pager's stdout to the terminal
            .stderr(Stdio::inherit());
        
        // Start the process
        let mut process = match cmd.spawn() {
            Ok(p) => p,
            Err(e) => {
                // Fallback to direct stdout if we can't start the pager
                self.enabled = false;
                return Ok(());
            }
        };
        
        // Get handle to stdin
        let stdin = match process.stdin.take() {
            Some(s) => s,
            None => {
                // Fallback to direct stdout if we can't get stdin handle
                self.enabled = false;
                return Ok(());
            }
        };
        
        self.process = Some(process);
        self.stdout = Some(stdin);
        
        Ok(())
    }
    
    /// Write text to the pager
    pub fn write(&mut self, text: &str) -> Result<(), Error> {
        // If pager is not enabled or user exited, don't write anything
        if !self.enabled || self.early_exit {
            return Ok(());
        }
        
        // If no stdout handle, write directly
        if self.stdout.is_none() {
            print!("{}", text);
            io::stdout().flush().map_err(|e| Error::IO(e))?;
            return Ok(());
        }
        
        // Write to pager
        if let Some(stdin) = &mut self.stdout {
            match stdin.write_all(text.as_bytes()) {
                Ok(_) => {
                    // Try to flush, but ignore broken pipe error
                    match stdin.flush() {
                        Ok(_) => {},
                        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                            // User exited pager, mark early exit
                            self.early_exit = true;
                            return Ok(());
                        },
                        Err(e) => return Err(Error::IO(e)),
                    }
                },
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                    // User exited pager, mark early exit
                    self.early_exit = true;
                    return Ok(());
                },
                Err(e) => {
                    return Err(Error::IO(e));
                }
            }
        }
        
        Ok(())
    }
    
    /// Close the pager and wait for the process to terminate
    pub fn close(&mut self) -> Result<(), Error> {
        // If user already exited pager, just clean up
        if self.early_exit {
            self.enabled = false;
            self.stdout.take();
            self.process.take();
            return Ok(());
        }
        
        // Only try to close if we have an active process
        if let Some(mut process) = self.process.take() {
            // First, drop the stdin handle to close the pager's input
            self.stdout.take();
            
            // Then wait for the process to terminate
            match process.wait() {
                Ok(_) => {},
                Err(e) => {
                    if e.kind() != io::ErrorKind::BrokenPipe {
                        // Only error on non-broken pipe errors
                        return Err(Error::IO(e));
                    }
                }
            }
        }
        
        // Make sure we're marked as disabled
        self.enabled = false;
        
        Ok(())
    }
    
    /// Check if the pager exited early (user pressed 'q')
    pub fn exited_early(&self) -> bool {
        self.early_exit
    }
    
    /// Disable the pager
    pub fn disable(&mut self) {
        self.enabled = false;
    }
    
    /// Check if the pager is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled && !self.early_exit
    }
}

impl Drop for Pager {
    fn drop(&mut self) {
        let _ = self.close();
    }
}