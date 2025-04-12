use std::path::{Path, PathBuf};
use std::fs;
use crate::errors::error::Error;
use crate::core::database::database::Database;
use crate::core::refs::Refs;
use crate::core::workspace::Workspace;
use crate::core::index::index::Index;
use std::collections::HashMap;
use crate::core::database::entry::DatabaseEntry;
use super::migration::Migration;

pub struct Repository {
    pub path: PathBuf,
    pub database: Database,
    pub refs: Refs,
    pub workspace: Workspace,
    pub index: Index,
}

impl Repository {
    pub fn new(path: &str) -> Result<Self, Error> {
        let path_buf = PathBuf::from(path).canonicalize().map_err(|e| {
            Error::PathResolution(format!("Failed to resolve path '{}': {}", path, e))
        })?;
        
        let git_path = path_buf.join(".ash");
        
        let db_path = git_path.join("objects");
        let index_path = git_path.join("index");
        
        Ok(Repository {
            workspace: Workspace::new(&path_buf),
            index: Index::new(index_path),
            database: Database::new(db_path),
            refs: Refs::new(&git_path),
            path: path_buf,
        })
    }

    pub fn create_git_directory(&self) -> Result<PathBuf, Error> {
        let git_path = self.path.join(".ash");
        self.create_directory(&git_path)?;
        Ok(git_path)
    }

    pub fn create_directory(&self, path: &Path) -> Result<(), Error> {
        fs::create_dir_all(path).map_err(|e| {
            Error::DirectoryCreation(format!(
                "Failed to create directory '{}': {}",
                path.display(),
                e
            ))
        })
    }
    
    /// Create a migration for moving between trees
    pub fn migration<'a>(&'a mut self, tree_diff: HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>) -> Migration<'a> {
        Migration::new(self, tree_diff)
    }

    pub fn tree_diff(&mut self, a_oid: Option<&str>, b_oid: Option<&str>) -> Result<HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>, Error> {
        // Create a default PathFilter that includes everything
        let path_filter = crate::core::path_filter::PathFilter::new();
        self.database.tree_diff(a_oid, b_oid, &path_filter)
    }
    
}