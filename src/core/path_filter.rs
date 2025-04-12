use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A trie structure for efficiently matching file paths
#[derive(Debug, Clone)]
struct Trie {
    /// Whether this node represents a path that was in the input list
    matched: bool,
    /// Child nodes in the trie
    children: HashMap<String, Trie>,
}

impl Trie {
    /// Create a new trie node
    fn new(matched: bool) -> Self {
        Trie {
            matched,
            children: HashMap::new(),
        }
    }
    
    /// Creates a trie from a list of paths
    fn from_paths(paths: &[PathBuf]) -> Self {
        let mut root = Trie::new(paths.is_empty());
        
        for path in paths {
            let mut trie = &mut root;
            
            for component in path.components() {
                let name = component.as_os_str().to_string_lossy().to_string();
                trie = trie.children.entry(name).or_insert_with(|| Trie::new(false));
            }
            
            // Mark the leaf node as matched
            trie.matched = true;
        }
        
        root
    }
}

/// A filter for selecting paths that match a given set of criteria
#[derive(Debug, Clone)]
pub struct PathFilter {
    /// The trie structure used for matching
    routes: Trie,
    /// The current path being examined
    path: PathBuf,
}

impl PathFilter {
    /// Create a new path filter with no filters (matches everything)
    pub fn new() -> Self {
        PathFilter {
            routes: Trie::new(true),
            path: PathBuf::new(),
        }
    }
    
    /// Build a path filter from a list of paths
    pub fn build(paths: &[PathBuf]) -> Self {
        PathFilter {
            routes: Trie::from_paths(paths),
            path: PathBuf::new(),
        }
    }
    
    /// Get the current path being examined
    pub fn path(&self) -> &Path {
        &self.path
    }
    
    /// Filter a set of entries, yielding only those that match the criteria
    pub fn filter_entries<'a, T>(&self, entries: &'a HashMap<String, T>) -> Vec<(&'a String, &'a T)> {
        let mut result = Vec::new();
        
        for (name, entry) in entries {
            if self.routes.matched || self.routes.children.contains_key(name) {
                result.push((name, entry));
            }
        }
        
        result
    }
    
    /// Create a new PathFilter by appending a path component
    pub fn join(&self, name: &str) -> Self {
        // If the current node is already matched, continue with the same routes
        // Otherwise, select the child route for the given name
        let next_routes = if self.routes.matched {
            self.routes.clone()
        } else {
            self.routes.children.get(name)
                .cloned()
                .unwrap_or_else(|| Trie::new(false))
        };
        
        PathFilter {
            routes: next_routes,
            path: self.path.join(name),
        }
    }
}

// Default implementation for PathFilter - matches everything
impl Default for PathFilter {
    fn default() -> Self {
        Self::new()
    }
}