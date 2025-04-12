use std::collections::HashMap;
use std::path::PathBuf;
use crate::core::file_mode::FileMode;
use crate::errors::error::Error;
use crate::core::database::database::Database;
use crate::core::database::tree::{Tree, TreeEntry};
use crate::core::database::commit::Commit;
use crate::core::database::entry::DatabaseEntry;
use crate::core::path_filter::PathFilter;

pub struct TreeDiff<'a> {
    database: &'a mut Database,
    pub changes: HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>,
}

impl<'a> TreeDiff<'a> {
    pub fn new(database: &'a mut Database) -> Self {
        TreeDiff {
            database,
            changes: HashMap::new(),
        }
    }
    
    /// Compare two object IDs (trees or commits) and find the differences
    pub fn compare_oids(&mut self, a: Option<&str>, b: Option<&str>, filter: &PathFilter) -> Result<(), Error> {
        // If both IDs are the same, there are no differences
        if a == b {
            return Ok(());
        }
        
        // Convert OIDs to trees
        let a_entries = if let Some(a_oid) = a {
            self.oid_to_tree_entries(a_oid)?
        } else {
            HashMap::new()
        };
        
        let b_entries = if let Some(b_oid) = b {
            self.oid_to_tree_entries(b_oid)?
        } else {
            HashMap::new()
        };
        
        // Find deletions and modifications
        self.detect_deletions(&a_entries, &b_entries, filter)?;
        
        // Find additions
        self.detect_additions(&a_entries, &b_entries, filter)?;
        
        Ok(())
    }
    
    /// Convert an OID to tree entries, handling both commit and tree objects
    fn oid_to_tree_entries(&mut self, oid: &str) -> Result<HashMap<String, DatabaseEntry>, Error> {
        let object = self.database.load(oid)?;
        
        match object.get_type() {
            "commit" => {
                if let Some(commit) = object.as_any().downcast_ref::<Commit>() {
                    // Get the tree OID from the commit and load it
                    let tree_oid = commit.get_tree();
                    let tree_obj = self.database.load(tree_oid)?;
                    
                    if let Some(tree) = tree_obj.as_any().downcast_ref::<Tree>() {
                        self.tree_to_entries(tree)
                    } else {
                        Err(Error::Generic(format!("Object {} is not a tree", tree_oid)))
                    }
                } else {
                    Err(Error::Generic(format!("Failed to downcast commit object {}", oid)))
                }
            },
            "tree" => {
                if let Some(tree) = object.as_any().downcast_ref::<Tree>() {
                    self.tree_to_entries(tree)
                } else {
                    Err(Error::Generic(format!("Failed to downcast tree object {}", oid)))
                }
            },
            _ => Err(Error::Generic(format!("Object {} is neither commit nor tree", oid))),
        }
    }
    
    /// Convert a tree object to a map of entries
    fn tree_to_entries(&self, tree: &Tree) -> Result<HashMap<String, DatabaseEntry>, Error> {
        let mut entries = HashMap::new();
        
        for (name, entry) in tree.get_entries() {
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    // Create a database entry for this blob
                    entries.insert(name.clone(), DatabaseEntry::new(
                        name.clone(),
                        oid.clone(),
                        &mode.to_octal_string(),
                    ));
                },
                TreeEntry::Tree(subtree) => {
                    // Create a database entry for this subtree
                    if let Some(subtree_oid) = subtree.get_oid() {
                        entries.insert(name.clone(), DatabaseEntry::new(
                            name.clone(),
                            subtree_oid.clone(),
                            "040000", // Directory mode
                        ));
                    }
                }
            }
        }
        
        Ok(entries)
    }
    
    /// Detect files that were deleted or modified
    fn detect_deletions(
        &mut self,
        a_entries: &HashMap<String, DatabaseEntry>,
        b_entries: &HashMap<String, DatabaseEntry>,
        filter: &PathFilter,
    ) -> Result<(), Error> {
        // Use the filter to get only the relevant entries
        let filtered_entries = filter.filter_entries(a_entries);
        
        for (name, a_entry) in filtered_entries {
            // Check if entry exists in b
            let b_entry = b_entries.get(name);
            
            // Skip if entries are identical
            if b_entry.is_some() && a_entry.get_oid() == b_entry.unwrap().get_oid() && 
               a_entry.get_mode() == b_entry.unwrap().get_mode() {
                continue;
            }
            
            // Check if both entries are trees
            let a_is_tree = a_entry.get_mode() == "040000";
            let b_is_tree = b_entry.map_or(false, |e| e.get_mode() == "040000");
            
            // Create a new filter for this path
            let sub_filter = filter.join(name);
            
            if a_is_tree && b_is_tree {
                // Both are trees, compare recursively
                self.compare_oids(
                    Some(a_entry.get_oid()),
                    Some(b_entry.unwrap().get_oid()),
                    &sub_filter,
                )?;
            } else {
                // Record the change
                self.changes.insert(
                    sub_filter.path().to_path_buf(),
                    (Some(a_entry.clone()), b_entry.cloned()),
                );
            }
        }
        
        Ok(())
    }
    
    /// Detect files that were added
    fn detect_additions(
        &mut self,
        a_entries: &HashMap<String, DatabaseEntry>,
        b_entries: &HashMap<String, DatabaseEntry>,
        filter: &PathFilter,
    ) -> Result<(), Error> {
        // Use the filter to get only the relevant entries
        let filtered_entries = filter.filter_entries(b_entries);
        
        for (name, b_entry) in filtered_entries {
            // Skip if entry exists in a (already handled by detect_deletions)
            if a_entries.contains_key(name) {
                continue;
            }
            
            // Create a new filter for this path
            let sub_filter = filter.join(name);
            
            // Check if entry is a tree by mode
            if b_entry.get_mode() == "040000" || FileMode::parse(b_entry.get_mode()).is_directory() {
                // It's a tree, compare recursively with empty a-side
                self.compare_oids(
                    None,
                    Some(b_entry.get_oid()),
                    &sub_filter,
                )?;
            } else {
                // Record the addition of a file
                self.changes.insert(
                    sub_filter.path().to_path_buf(),
                    (None, Some(b_entry.clone())),
                );
            }
        }
        
        Ok(())
    }
}