// src/core/database/tree.rs with FileMode struct
use crate::core::database::database::Database;
use crate::core::database::entry::DatabaseEntry;
use crate::core::file_mode::FileMode;
use super::database::GitObject;
use crate::errors::error::Error;
use itertools::Itertools;
use std::collections::HashMap;
use std::any::Any;
use std::path::PathBuf;

#[derive(Debug)]
#[derive(Clone)]
pub struct Tree {
    oid: Option<String>,
    entries: HashMap<String, TreeEntry>,
}

#[derive(Debug)]
#[derive(Clone)]
pub enum TreeEntry {
    Blob(String, FileMode), // oid, mode
    Tree(Box<Tree>),
}

// Constants for mode
pub const TREE_MODE: FileMode = FileMode::DIRECTORY;
pub const REGULAR_MODE: FileMode = FileMode::REGULAR;
pub const EXECUTABLE_MODE: FileMode = FileMode::EXECUTABLE;

impl GitObject for Tree {
    fn get_type(&self) -> &str {
        "tree"
    }

    fn to_bytes(&self) -> Vec<u8> {
        // Existing implementation...
        let mut result = Vec::new();
        
        // Sort entries by name to ensure consistent output
        for (name, entry) in self.entries.iter().sorted_by_key(|(name, _)| *name) {
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    // Format: "<mode> <name>\0<sha1>"
                    let mode_str = mode.to_octal_string();
                    let entry_header = format!("{} {}\0", mode_str, name);
                    result.extend_from_slice(entry_header.as_bytes());
                    
                    // Add binary OID (20 bytes)
                    if let Ok(oid_bytes) = hex::decode(oid) {
                        if oid_bytes.len() == 20 {
                            result.extend_from_slice(&oid_bytes);
                        } else {
                            println!("Warning: OID {} is not 20 bytes ({}), padding", oid, oid_bytes.len());
                            // Pad or truncate to 20 bytes
                            let mut fixed_oid = vec![0; 20];
                            let copy_len = std::cmp::min(oid_bytes.len(), 20);
                            fixed_oid[..copy_len].copy_from_slice(&oid_bytes[..copy_len]);
                            result.extend_from_slice(&fixed_oid);
                        }
                    } else {
                        println!("Warning: Failed to decode OID: {}", oid);
                        // Use a placeholder OID (20 zeros)
                        result.extend_from_slice(&[0; 20]);
                    }
                },
                TreeEntry::Tree(subtree) => {
                    // For tree entries, ALWAYS mark them with tree mode (040000)
                    // This is critical - using the correct type identifier for directories
                    let entry_header = format!("{} {}\0", TREE_MODE, name);
                    result.extend_from_slice(entry_header.as_bytes());
                    
                    // Add binary OID (20 bytes)
                    if let Some(oid) = &subtree.oid {
                        if let Ok(oid_bytes) = hex::decode(oid) {
                            if oid_bytes.len() == 20 {
                                result.extend_from_slice(&oid_bytes);
                            } else {
                                println!("Warning: Tree OID {} is not 20 bytes ({}), padding", oid, oid_bytes.len());
                                // Pad or truncate to 20 bytes
                                let mut fixed_oid = vec![0; 20];
                                let copy_len = std::cmp::min(oid_bytes.len(), 20);
                                fixed_oid[..copy_len].copy_from_slice(&oid_bytes[..copy_len]);
                                result.extend_from_slice(&fixed_oid);
                            }
                        } else {
                            println!("Warning: Failed to decode tree OID: {}", oid);
                            // Use a placeholder OID (20 zeros)
                            result.extend_from_slice(&[0; 20]);
                        }
                    } else {
                        println!("Warning: Tree entry has no OID: {}", name);
                        // Use a placeholder OID (20 zeros)
                        result.extend_from_slice(&[0; 20]);
                    }
                }
            }
        }
        
        result
    }

    fn set_oid(&mut self, oid: String) {
        self.oid = Some(oid);
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    // Implementation of clone_box to properly clone the object
    fn clone_box(&self) -> Box<dyn GitObject> {
        Box::new(self.clone())
    }
}

impl Tree {
    pub fn new() -> Self {
        Tree {
            oid: None,
            entries: HashMap::new(),
        }
    }
    
    pub fn get_entries(&self) -> &HashMap<String, TreeEntry> {
        &self.entries
    }
    
    pub fn build<'a, I>(entries: I) -> Result<Self, Error>
    where
        I: Iterator<Item = &'a DatabaseEntry>,
    {
        let mut root = Tree::new();
        
        for entry in entries {
            let path_str = entry.get_name();
            let path = PathBuf::from(path_str);
            
            // Split the path into components
            let components: Vec<_> = path.components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            
            if components.is_empty() {
                println!("Warning: Empty path components for {}", path_str);
                continue;
            }
            
            println!("Processing entry: {}", path_str);
            
            // Handle top-level file
            if components.len() == 1 {
                let mode = FileMode::parse(entry.get_mode());
                
                root.entries.insert(
                    components[0].clone(),
                    TreeEntry::Blob(entry.get_oid().to_string(), mode)
                );
                
                println!("Added top-level file: {}", components[0]);
                continue;
            }
            
            // For nested paths, navigate directories
            let filename = components.last().unwrap().clone();
            let dir_components = &components[..components.len() - 1];
            
            // Start at root
            let mut current = &mut root;
            let mut current_path = Vec::new();
            
            for dir in dir_components {
                current_path.push(dir.clone());
                let dir_str = current_path.join("/");
                println!("Creating/navigating directory: {}", dir_str);
                
                // Check if we need to create a directory
                let need_new_dir = match current.entries.get(dir) {
                    Some(TreeEntry::Tree(_)) => false, // Directory exists
                    Some(TreeEntry::Blob(_, _)) => {
                        // Path conflict - file exists where directory is needed
                        return Err(Error::Generic(format!(
                            "Path conflict: '{}' exists as a file but is also used as a directory in '{}'",
                            dir_str, path_str
                        )));
                    },
                    None => true, // Need to create directory
                };
                
                if need_new_dir {
                    println!("Creating new directory: {}", dir);
                    current.entries.insert(
                        dir.clone(),
                        TreeEntry::Tree(Box::new(Tree::new()))
                    );
                }
                
                // Get mutable reference to subdirectory
                if let Some(TreeEntry::Tree(subtree)) = current.entries.get_mut(dir) {
                    current = subtree;
                } else {
                    return Err(Error::Generic(format!(
                        "Unexpected error navigating to directory: {}", dir_str
                    )));
                }
            }
            
            // Add file at current position
            let mode = FileMode::parse(entry.get_mode());
            
            println!("Adding file: {} to directory: {}", filename, current_path.join("/"));
            current.entries.insert(
                filename.clone(),
                TreeEntry::Blob(entry.get_oid().to_string(), mode)
            );
        }
        
        // Print final tree structure for debugging
        println!("Final tree structure:");
        root.dump_structure("  ");
        
        Ok(root)
    }

    pub fn add_entry(&mut self, entry: &DatabaseEntry) -> Result<(), Error> {
        let parent_dirs = entry.parent_directories();
        let basename = entry.basename();
        
        // Ensure all parent directories exist
        let mut current = self;
        
        for dir in parent_dirs {
            let dir_name = dir.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            
            if dir_name.is_empty() {
                continue;
            }
            
            // Verificăm dacă avem nevoie să adăugăm un nou subdirector sau să folosim unul existent
            let entry_type = {
                if let Some(entry) = current.entries.get(&dir_name) {
                    match entry {
                        TreeEntry::Tree(_) => None, // E deja un arbore, e ok
                        _ => return Err(Error::Generic(format!(
                            "Entry '{}' conflicts with existing blob entry",
                            dir_name
                        ))),
                    }
                } else {
                    // Trebuie să adăugăm un nou subdirector
                    Some(TreeEntry::Tree(Box::new(Tree::new())))
                }
            };
            
            // Adăugăm subdirectorul dacă e necesar
            if let Some(tree_entry) = entry_type {
                current.entries.insert(dir_name.clone(), tree_entry);
            }
            
            // Obținem o referință la subdirector
            if let Some(TreeEntry::Tree(tree)) = current.entries.get_mut(&dir_name) {
                current = tree;
            } else {
                unreachable!("Subdirectory should exist at this point");
            }
        }
        
        // Add the file entry to the current tree
        let mode = FileMode::parse(entry.get_mode());
        current.entries.insert(
            basename,
            TreeEntry::Blob(entry.get_oid().to_string(), mode)
        );
        
        Ok(())
    }
    
    // În tree.rs:
    pub fn traverse<F>(&mut self, mut func: F) -> Result<(), Error>
    where
        F: FnMut(&mut Tree) -> Result<(), Error>
    {
        // Process subtrees first (bottom-up)
        let mut names_to_process = Vec::new();
        
        // Collect names of all tree entries
        for (name, _) in &self.entries {
            names_to_process.push(name.clone());
        }
        
        // Process each entry
        for name in names_to_process {
            if let Some(TreeEntry::Tree(subtree)) = self.entries.get_mut(&name) {
                println!("Traversing subtree: {}", name);
                // Process subtree recursively - using traverse_internal
                subtree.traverse_internal(&mut func)?;
                
                // Verify subtree has OID set after traversal
                if subtree.oid.is_none() {
                    println!("Warning: Subtree {} has no OID after traversal", name);
                } else {
                    println!("Subtree {} has OID {} after traversal", name, subtree.oid.as_ref().unwrap());
                }
            }
        }
        
        // Finally, process this tree
        println!("Processing tree with {} entries", self.entries.len());
        func(self)?;
        
        // Verify this tree has OID set
        if self.oid.is_none() {
            println!("Warning: Tree has no OID after processing");
        } else {
            println!("Tree has OID {} after processing", self.oid.as_ref().unwrap());
        }
        
        Ok(())
    }

    fn traverse_internal<F>(&mut self, func: &mut F) -> Result<(), Error>
    where
        F: FnMut(&mut Tree) -> Result<(), Error>
    {
        // Process subtrees first (bottom-up)
        let mut names_to_process = Vec::new();
        
        // Collect names of all tree entries
        for (name, _) in &self.entries {
            names_to_process.push(name.clone());
        }
        
        // Process each entry
        for name in names_to_process {
            if let Some(TreeEntry::Tree(subtree)) = self.entries.get_mut(&name) {
                println!("Traversing internal subtree: {}", name);
                // Process subtree recursively
                subtree.traverse_internal(func)?;
                
                // Verify subtree has OID set after traversal
                if subtree.oid.is_none() {
                    println!("Warning: Internal subtree {} has no OID after traversal", name);
                } else {
                    println!("Internal subtree {} has OID {} after traversal", 
                            name, subtree.oid.as_ref().unwrap());
                }
            }
        }
        
        // Finally, process this tree
        println!("Processing internal tree with {} entries", self.entries.len());
        let result = func(self);
        
        // Verify OID is set after processing
        if self.oid.is_none() {
            println!("Warning: Internal tree has no OID after processing");
        } else {
            println!("Internal tree has OID {} after processing", self.oid.as_ref().unwrap());
        }
        
        result
    }

    pub fn get_oid(&self) -> Option<&String> {
        self.oid.as_ref()
    }
    
    /// Parsează un tree dintr-un șir de bytes
    /// Improved parsing of a tree from its binary representation
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut tree = Tree::new();
        let mut pos = 0;
        
        while pos < data.len() {
            // Find first space for mode
            if let Some(space_pos) = data[pos..].iter().position(|&b| b == b' ') {
                // Parse mode as octal
                let mode_str = match std::str::from_utf8(&data[pos..pos+space_pos]) {
                    Ok(s) => s,
                    Err(_) => return Err(Error::Generic("Invalid UTF-8 in mode".to_string())),
                };
                
                let mode = FileMode::parse(mode_str);
                pos += space_pos + 1;
                
                // Find null terminator for name
                if let Some(null_pos) = data[pos..].iter().position(|&b| b == 0) {
                    // Extract name
                    let name = match std::str::from_utf8(&data[pos..pos+null_pos]) {
                        Ok(s) => s,
                        Err(_) => return Err(Error::Generic("Invalid UTF-8 in name".to_string())),
                    };
                    
                    pos += null_pos + 1;
                    
                    // Ensure we have enough bytes for OID (20)
                    if pos + 20 > data.len() {
                        return Err(Error::Generic("Invalid tree format: truncated SHA-1".to_string()));
                    }
                    
                    // Extract OID as hex string
                    let oid = hex::encode(&data[pos..pos+20]);
                    pos += 20;
                    
                    // MODIFICAREA CRUCIALĂ - verifică modul pentru a determina tipul intrării
                    if mode.is_directory() {
                        // Aceasta este o intrare de director
                        println!("Tree parse: Found directory entry: {} -> {} (mode {})", name, oid, mode);
                        let mut subtree = Tree::new();
                        subtree.set_oid(oid);
                        tree.entries.insert(name.to_string(), TreeEntry::Tree(Box::new(subtree)));
                    } else {
                        // Aceasta este o intrare normală de fișier
                        println!("Tree parse: Found file entry: {} -> {} (mode {})", name, oid, mode);
                        tree.entries.insert(name.to_string(), TreeEntry::Blob(oid, mode));
                    }
                } else {
                    return Err(Error::Generic("Invalid tree format: missing null terminator after name".to_string()));
                }
            } else {
                return Err(Error::Generic("Invalid tree format: missing space after mode".to_string()));
            }
        }
        
        Ok(tree)
    }
     
    pub fn insert_entry(&mut self, name: String, entry: TreeEntry) {
        self.entries.insert(name, entry);
    }
    
    // Dacă e nevoie și de o metodă pentru a obține o intrare după nume
    pub fn get_entry(&self, name: &str) -> Option<&TreeEntry> {
        self.entries.get(name)
    }
    
    // Dacă e nevoie și de acces mutabil la o intrare
    pub fn get_entry_mut(&mut self, name: &str) -> Option<&mut TreeEntry> {
        self.entries.get_mut(name)
    }
    
    pub fn dump_structure(&self, prefix: &str) {
        println!("{}Tree Structure:", prefix);
        self.dump_entries(prefix, "");
    }
    
    fn dump_entries(&self, prefix: &str, path: &str) {
        for (name, entry) in &self.entries {
            let entry_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", path, name)
            };
            
            match entry {
                TreeEntry::Blob(oid, mode) => {
                    println!("{}{} (blob, mode {}) -> {}", prefix, entry_path, mode, oid);
                },
                TreeEntry::Tree(subtree) => {
                    if let Some(oid) = subtree.get_oid() {
                        println!("{}{} (tree) -> {}", prefix, entry_path, oid);
                        subtree.dump_entries(prefix, &entry_path);
                    } else {
                        println!("{}{} (tree) -> <no OID>", prefix, entry_path);
                        subtree.dump_entries(prefix, &entry_path);
                    }
                }
            }
        }
    }
    
    pub fn inspect_tree_structure(database: &mut Database, tree_oid: &str, depth: usize) -> Result<(), Error> {
        let indent = "  ".repeat(depth);
        println!("{}Inspecting tree: {}", indent, tree_oid);
        
        // Load the object
        let obj = database.load(tree_oid)?;
        println!("{}Object type: {}", indent, obj.get_type());
        
        // If it's a tree, process it directly
        if let Some(tree) = obj.as_any().downcast_ref::<Tree>() {
            println!("{}Tree has {} entries:", indent, tree.get_entries().len());
            
            for (name, entry) in tree.get_entries() {
                match entry {
                    TreeEntry::Blob(blob_oid, mode) => {
                        if mode.is_directory() {
                            println!("{}+ {} (directory stored as blob) -> {}", 
                                    indent, name, blob_oid);
                            // Recursively inspect this directory
                            Self::inspect_tree_structure(database, blob_oid, depth + 1)?;
                        } else {
                            println!("{}+ {} (file, mode {}) -> {}", 
                                    indent, name, mode, blob_oid);
                        }
                    },
                    TreeEntry::Tree(subtree) => {
                        if let Some(subtree_oid) = subtree.get_oid() {
                            println!("{}+ {} (directory) -> {}", indent, name, subtree_oid);
                            // Recursively inspect this directory
                            Self::inspect_tree_structure(database, subtree_oid, depth + 1)?;
                        } else {
                            println!("{}+ {} (directory without OID)", indent, name);
                        }
                    }
                }
            }
            
            return Ok(());
        }
        
        // If it's a blob, try to parse it as a tree
        if obj.get_type() == "blob" {
            println!("{}Blob, attempting to parse as tree...", indent);
            
            let blob_data = obj.to_bytes();
            match Tree::parse(&blob_data) {
                Ok(tree) => {
                    println!("{}Successfully parsed as tree with {} entries:", 
                            indent, tree.get_entries().len());
                    
                    for (name, entry) in tree.get_entries() {
                        match entry {
                            TreeEntry::Blob(blob_oid, mode) => {
                                if mode.is_directory() {
                                    println!("{}+ {} (directory stored as blob) -> {}", 
                                            indent, name, blob_oid);
                                    // Recursively inspect this directory
                                    Self::inspect_tree_structure(database, blob_oid, depth + 1)?;
                                } else {
                                    println!("{}+ {} (file, mode {}) -> {}", 
                                            indent, name, mode, blob_oid);
                                }
                            },
                            TreeEntry::Tree(subtree) => {
                                if let Some(subtree_oid) = subtree.get_oid() {
                                    println!("{}+ {} (directory) -> {}", indent, name, subtree_oid);
                                    // Recursively inspect this directory
                                    Self::inspect_tree_structure(database, subtree_oid, depth + 1)?;
                                } else {
                                    println!("{}+ {} (directory without OID)", indent, name);
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    println!("{}Failed to parse as tree: {}", indent, e);
                }
            }
            
            return Ok(());
        }
        
        println!("{}Neither a tree nor a parseable blob", indent);
        Ok(())
    }
}