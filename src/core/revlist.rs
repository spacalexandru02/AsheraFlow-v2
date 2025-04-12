// src/core/revlist.rs with all clone_box fixes

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use crate::core::database::database::{Database, GitObject};
use crate::core::database::commit::Commit;
use crate::core::revision::{HEAD, COMMIT};
use crate::core::path_filter::PathFilter;
use crate::errors::error::Error;

/// RevList handles traversing commit history and filtering commits
/// based on various criteria (date, path, etc.)
pub struct RevList<'a> {
    database: &'a mut Database,
    
    // Commit storage and tracking
    commits: HashMap<String, Box<dyn GitObject>>,
    flags: HashMap<String, HashSet<Flag>>,
    
    // Queue of commits to process
    queue: VecDeque<Box<dyn GitObject>>,
    output: Vec<Box<dyn GitObject>>,
    
    // Path filtering
    path_filter: PathFilter,
    
    // Limitation flags
    limited: bool,
    
    // Diffs cache
    diffs: HashMap<(Option<String>, String), HashMap<PathBuf, (Option<String>, Option<String>)>>,
}

/// Flags that can be associated with commits during traversal
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum Flag {
    Seen,        // Commit has been seen
    Added,       // Commit has been added to the traversal queue
    Uninteresting, // Commit is excluded from output
    TreeSame,    // Commit doesn't change anything we're interested in (for path filtering)
}

impl<'a> RevList<'a> {
    /// Create a new RevList with the given revisions
    pub fn new(database: &'a mut Database, revisions: &[String]) -> Result<Self, Error> {
        let mut revlist = RevList {
            database,
            commits: HashMap::new(),
            flags: HashMap::new(),
            queue: VecDeque::new(),
            output: Vec::new(),
            path_filter: PathFilter::new(),
            limited: false,
            diffs: HashMap::new(),
        };
        
        let mut has_revisions = false;
        let mut path_filters = Vec::new();
        
        // Process all revisions
        for rev in revisions {
            // Check if this is a path that exists in the workspace
            let path = PathBuf::from(rev);
            if path.exists() {
                path_filters.push(path);
                continue;
            }
            
            // Try to handle it as a revision
            revlist.handle_revision(rev)?;
            has_revisions = true;
        }
        
        // Initialize path filter if any paths were specified
        if !path_filters.is_empty() {
            revlist.path_filter = PathFilter::build(&path_filters);
        }
        
        // If no revisions were given, use HEAD
        if !has_revisions {
            revlist.handle_revision(HEAD)?;
        }
        
        // If using path filtering with limited revisions, perform filtering
        if revlist.limited && !path_filters.is_empty() {
            revlist.limit_list()?;
        }
        
        Ok(revlist)
    }
    
    /// Handle a single revision string
    fn handle_revision(&mut self, rev: &str) -> Result<(), Error> {
        // Check for range notation: A..B
        if let Some(pos) = rev.find("..") {
            let start = &rev[..pos];
            let end = &rev[pos+2..];
            
            // Handle empty endpoints (default to HEAD)
            let start = if start.is_empty() { HEAD } else { start };
            let end = if end.is_empty() { HEAD } else { end };
            
            // Mark the start as uninteresting, end as interesting
            self.set_start_point(start, false)?;
            self.set_start_point(end, true)?;
            
            return Ok(());
        }
        
        // Check for exclude notation: ^A
        if rev.starts_with('^') {
            let excluded = &rev[1..];
            self.set_start_point(excluded, false)?;
            return Ok(());
        }
        
        // Normal revision - mark as interesting
        self.set_start_point(rev, true)?;
        
        Ok(())
    }
    
    /// Set a starting point for the revision walk
    fn set_start_point(&mut self, rev: &str, interesting: bool) -> Result<(), Error> {
        // Resolve the revision to a commit OID
        // This should be done with a Repository but for this example we'll fake it
        // Direct implementation would call repo.database.load()
        let oid = match rev {
            HEAD => {
                // In a real implementation, would resolve HEAD to a commit OID
                // For now, just return a placeholder
                "HEAD_COMMIT_OID".to_string()
            },
            _ => {
                // Fake resolution for other refs
                format!("{}_RESOLVED", rev)
            }
        };
        
        // Load the commit
        let commit = self.load_commit(&oid)?;
        
        // Add to the queue - use clone_box instead of clone
        self.enqueue_commit(commit.clone_box());
        
        // If not interesting, mark as uninteresting and propagate to parents
        if !interesting {
            self.limited = true;
            self.mark(&oid, Flag::Uninteresting);
            self.mark_parents_uninteresting(&oid)?;
        }
        
        Ok(())
    }
    
    /// Mark a commit with a flag
    fn mark(&mut self, oid: &str, flag: Flag) -> bool {
        let flags = self.flags.entry(oid.to_string()).or_insert_with(HashSet::new);
        flags.insert(flag)
    }
    
    /// Check if a commit is marked with a flag
    fn is_marked(&self, oid: &str, flag: &Flag) -> bool {
        self.flags.get(oid)
            .map_or(false, |flags| flags.contains(flag))
    }
    
    /// Mark all parents of a commit as uninteresting
    fn mark_parents_uninteresting(&mut self, oid: &str) -> Result<(), Error> {
        let mut current_oid = oid.to_string();
        
        while !current_oid.is_empty() {
            let commit = self.load_commit(&current_oid)?;
            
            if let Some(parent) = self.get_parent(&commit) {
                if !self.mark(&parent, Flag::Uninteresting) {
                    // If parent was already marked, we can stop
                    break;
                }
                current_oid = parent;
            } else {
                // No parent, we're done
                break;
            }
        }
        
        Ok(())
    }
    
    /// Add a commit to the processing queue
    fn enqueue_commit(&mut self, commit: Box<dyn GitObject>) {
        let oid = self.get_oid(&commit);
        
        // Skip if already seen
        if !self.mark(&oid, Flag::Seen) {
            return;
        }
        
        self.queue.push_back(commit);
    }
    
    /// Process the queue and limit to interesting commits
    fn limit_list(&mut self) -> Result<(), Error> {
        while self.still_interesting()? {
            if let Some(commit) = self.queue.pop_front() {
                self.add_parents(&commit)?;
                
                let oid = self.get_oid(&commit);
                
                if !self.is_marked(&oid, &Flag::Uninteresting) {
                    self.output.push(commit.clone_box());
                }
            }
        }
        
        // Replace queue with output - manual copy using clone_box instead of clone
        self.queue.clear();
        for commit in &self.output {
            self.queue.push_back(commit.clone_box());
        }
        
        Ok(())
    }
    
    /// Check if there are still interesting commits to process
    fn still_interesting(&self) -> Result<bool, Error> {
        // If queue is empty, we're done
        if self.queue.is_empty() {
            return Ok(false);
        }
        
        // If we have output and the newest commit in the queue is older than
        // the oldest commit in the output, continue processing
        if !self.output.is_empty() {
            let oldest_out = self.output.last().unwrap();
            let oldest_out_date = self.get_commit_date(oldest_out)?;
            
            let newest_in = self.queue.front().unwrap();
            let newest_in_date = self.get_commit_date(newest_in)?;
            
            if oldest_out_date <= newest_in_date {
                return Ok(true);
            }
        }
        
        // If any commit in the queue is not marked uninteresting, continue
        for commit in &self.queue {
            let oid = self.get_oid(commit);
            if !self.is_marked(&oid, &Flag::Uninteresting) {
                return Ok(true);
            }
        }
        
        Ok(false)
    }
    
    /// Add parents of a commit to the queue
    fn add_parents(&mut self, commit: &Box<dyn GitObject>) -> Result<(), Error> {
        let oid = self.get_oid(commit);
        
        // Skip if already processed
        if !self.mark(&oid, Flag::Added) {
            return Ok(());
        }
        
        // Get parent commit
        if let Some(parent_oid) = self.get_parent(commit) {
            // If current commit is uninteresting, mark parent as uninteresting
            if self.is_marked(&oid, &Flag::Uninteresting) {
                self.mark(&parent_oid, Flag::Uninteresting);
                self.mark_parents_uninteresting(&parent_oid)?;
            }
            
            // If path filtering is active, simplify commit
            let original_commit = self.load_commit(&oid)?;
            if !self.path_filter.path().as_os_str().is_empty() {
                self.simplify_commit(&original_commit)?;
            }
            
            // Add parent to queue
            let parent_commit = self.load_commit(&parent_oid)?;
            self.enqueue_commit(parent_commit.clone_box());
        }
        
        Ok(())
    }
    
    /// Simplify a commit based on path filter
    fn simplify_commit(&mut self, commit: &Box<dyn GitObject>) -> Result<(), Error> {
        let oid = self.get_oid(commit);
        let parent_oid = self.get_parent(commit);
        
        // If there's no path filter or no parent, nothing to do
        if self.path_filter.path().as_os_str().is_empty() || parent_oid.is_none() {
            return Ok(());
        }
        
        // Get the diff between this commit and its parent
        let diff = self.tree_diff(parent_oid.as_deref(), &oid)?;
        
        // If no changes to paths we care about, mark as TreeSame
        if diff.is_empty() {
            self.mark(&oid, Flag::TreeSame);
        }
        
        Ok(())
    }
    
    /// Get diff between two trees, filtered by path_filter
    fn tree_diff(&mut self, a: Option<&str>, b: &str) -> Result<HashMap<PathBuf, (Option<String>, Option<String>)>, Error> {
        let key = (a.map(|s| s.to_string()), b.to_string());
        
        // Return cached result if available
        if let Some(diff) = self.diffs.get(&key) {
            return Ok(diff.clone());
        }
        
        // Calculate diff and store in cache
        let diff = self.database.tree_diff(a, Some(b), &self.path_filter)?;
        
        // Convert DatabaseEntry to String (OID) for simplified storage
        let mut result = HashMap::new();
        for (path, (old, new)) in diff {
            let old_oid = old.map(|e| e.get_oid().to_string());
            let new_oid = new.map(|e| e.get_oid().to_string());
            result.insert(path, (old_oid, new_oid));
        }
        
        self.diffs.insert(key, result.clone());
        Ok(result)
    }
    
    /// Get diff between two trees for a specific commit
    pub fn get_diff_for_commit(&mut self, commit: &Box<dyn GitObject>) -> Result<HashMap<PathBuf, (Option<String>, Option<String>)>, Error> {
        let oid = self.get_oid(commit);
        let parent = self.get_parent(commit);
        
        self.tree_diff(parent.as_deref(), &oid)
    }
    
    /// Load a commit by OID
    fn load_commit(&mut self, oid: &str) -> Result<Box<dyn GitObject>, Error> {
        // Return cached commit if available
        if let Some(commit) = self.commits.get(oid) {
            // Use clone_box instead of clone
            return Ok(commit.clone_box());
        }
        
        // Load from database
        let commit = self.database.load(oid)?;
        
        // Check if it's a commit
        if commit.get_type() != COMMIT {
            return Err(Error::Generic(format!("Object {} is not a commit", oid)));
        }
        
        // Cache and return
        // Use clone_box instead of clone
        self.commits.insert(oid.to_string(), commit.clone_box());
        Ok(commit)
    }
    
    /// Helper to get OID from a commit
    fn get_oid(&self, commit: &Box<dyn GitObject>) -> String {
        if let Some(commit) = commit.as_any().downcast_ref::<Commit>() {
            // Fix: Use cloned().unwrap_or_default() instead of unwrap_or_default().clone()
            commit.get_oid().cloned().unwrap_or_default()
        } else {
            String::new()
        }
    }
    
    /// Helper to get parent OID from a commit
    fn get_parent(&self, commit: &Box<dyn GitObject>) -> Option<String> {
        if let Some(commit) = commit.as_any().downcast_ref::<Commit>() {
            commit.get_parent().cloned()
        } else {
            None
        }
    }
    
    /// Helper to get commit date
    fn get_commit_date(&self, commit: &Box<dyn GitObject>) -> Result<i64, Error> {
        if let Some(commit) = commit.as_any().downcast_ref::<Commit>() {
            if let Some(author) = commit.get_author() {
                return Ok(author.timestamp.timestamp());
            }
        }
        
        Err(Error::Generic("Failed to get commit date".to_string()))
    }
    
    /// Iterate through the commits in the history
    pub fn for_each<F>(&mut self, mut f: F) -> Result<(), Error>
    where
        F: FnMut(&Box<dyn GitObject>) -> Result<(), Error>
    {
        while let Some(commit) = self.queue.pop_front() {
            self.add_parents(&commit)?;
            
            let oid = self.get_oid(&commit);
            
            // Skip uninteresting commits
            if self.is_marked(&oid, &Flag::Uninteresting) {
                continue;
            }
            
            // Skip commits that don't change paths we care about
            if self.is_marked(&oid, &Flag::TreeSame) {
                continue;
            }
            
            // Process the commit
            f(&commit)?;
        }
        
        Ok(())
    }
}