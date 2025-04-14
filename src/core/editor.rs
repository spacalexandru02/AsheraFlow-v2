use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use crate::errors::error::Error;

const DEFAULT_EDITOR: &str = "vi";

#[derive(Debug)]
pub struct Editor {
    path: PathBuf,
    command: String,
    closed: bool,
    file: File,
}

impl Editor {
    pub fn new(path: PathBuf, command: Option<String>) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| Error::Generic(format!("Failed to open file for editing: {}", e)))?;

        Ok(Self {
            path,
            command: command.unwrap_or_else(|| DEFAULT_EDITOR.to_owned()),
            closed: false,
            file,
        })
    }

    pub fn edit<F>(path: PathBuf, command: Option<String>, f: F) -> Result<Option<String>, Error>
    where
        F: FnOnce(&mut Editor) -> Result<(), Error>,
    {
        let mut editor = Editor::new(path, command)?;
        f(&mut editor)?;
        editor.edit_file()
    }

    pub fn write(&mut self, string: &str) -> Result<(), Error> {
        if self.closed {
            return Ok(());
        }
        self.file.write_all(string.as_bytes())
            .map_err(|e| Error::Generic(format!("Failed to write to file: {}", e)))?;
        self.file.write_all(b"\n")
            .map_err(|e| Error::Generic(format!("Failed to write newline to file: {}", e)))?;

        Ok(())
    }

    pub fn note(&mut self, string: &str) -> Result<(), Error> {
        if self.closed {
            return Ok(());
        }
        for line in string.lines() {
            write!(self.file, "# {}\n", line)
                .map_err(|e| Error::Generic(format!("Failed to write note to file: {}", e)))?;
        }

        Ok(())
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    pub fn edit_file(&mut self) -> Result<Option<String>, Error> {
        // Close the file before launching the editor
        drop(std::mem::replace(&mut self.file, unsafe { std::mem::zeroed() }));

        if self.closed {
            return Ok(None);
        }

        // Split the command for safer execution
        let parts: Vec<&str> = self.command.split_whitespace().collect();
        if parts.is_empty() {
            return Err(Error::Generic("Empty editor command".to_string()));
        }

        let mut cmd = Command::new(parts[0]);
        for part in parts.iter().skip(1) {
            cmd.arg(part);
        }
        cmd.arg(&self.path);

        let status = cmd.status()
            .map_err(|e| Error::Generic(format!("Failed to run editor: {}", e)))?;

        if !status.success() {
            return Err(Error::Generic(format!("Editor exited with status: {}", status)));
        }

        // Read the file and remove comments
        let content = fs::read_to_string(&self.path)
            .map_err(|e| Error::Generic(format!("Failed to read edited file: {}", e)))?;

        Ok(self.remove_notes(content))
    }

    fn remove_notes(&self, content: String) -> Option<String> {
        let lines: Vec<String> = content.lines()
            .filter(|line| !line.starts_with('#'))
            .map(String::from)
            .collect();

        if lines.iter().all(|line| line.trim().is_empty()) {
            None
        } else {
            Some(format!("{}\n", lines.join("\n").trim()))
        }
    }
}
