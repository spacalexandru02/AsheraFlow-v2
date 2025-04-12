use std::collections::{HashMap, HashSet, VecDeque};
use crate::core::database::database::Database;
use crate::core::database::commit::Commit;
use crate::errors::error::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Flag {
    Parent1,
    Parent2,
    Result,
    Stale,
}

pub struct CommonAncestors<'a> {
    database: &'a mut Database,
    flags: HashMap<String, HashSet<Flag>>,
    queue: VecDeque<String>, // Store OIDs instead of objects
    results: VecDeque<String>, // Store OIDs instead of objects
}

impl<'a> CommonAncestors<'a> {
    pub fn new(database: &'a mut Database, one: &str, twos: &[&str]) -> Result<Self, Error> {
        let mut queue = VecDeque::new();
        let mut flags = HashMap::new();

        // Verify one is a commit
        database.load(one)?;
        
        // Mark one with Parent1 flag
        queue.push_back(one.to_string());
        let mut one_flags = HashSet::new();
        one_flags.insert(Flag::Parent1);
        flags.insert(one.to_string(), one_flags);

        // Process each two commit
        for two in twos {
            // Verify two is a commit
            database.load(two)?;
            
            // Add to queue and mark with Parent2 flag
            queue.push_back(two.to_string());
            
            // Use entry API to handle the case where one and two are the same
            let two_flags = flags.entry(two.to_string()).or_insert_with(HashSet::new);
            two_flags.insert(Flag::Parent2);
        }

        Ok(Self {
            database,
            flags,
            queue,
            results: VecDeque::new(),
        })
    }

    pub fn find(&mut self) -> Result<Vec<String>, Error> {
        // Set of flags that indicate both parents are present
        let both_parents: HashSet<Flag> = {
            let mut set = HashSet::new();
            set.insert(Flag::Parent1);
            set.insert(Flag::Parent2);
            set
        };

        // Process the queue until all remaining commits are stale
        while !self.all_stale() {
            // Pop the front commit OID from the queue
            if let Some(commit_oid) = self.queue.pop_front() {
                // Load the commit
                let commit_obj = self.database.load(&commit_oid)?;
                if let Some(commit) = commit_obj.as_any().downcast_ref::<Commit>() {
                    let flags = self.flags.get_mut(&commit_oid).unwrap();

                    // If commit has both Parent1 and Parent2 flags, it's a common ancestor
                    let is_common_ancestor = flags.contains(&Flag::Parent1) && flags.contains(&Flag::Parent2);
                    if is_common_ancestor {
                        flags.insert(Flag::Result);
                        self.results.push_back(commit_oid.clone());
                        
                        // Mark parents as stale
                        let mut flags_clone = flags.clone();
                        flags_clone.insert(Flag::Stale);
                        self.add_parents(commit, &commit_oid, &flags_clone)?;
                    } else {
                        // Not a common ancestor, propagate flags to parents
                        let flags_clone = flags.clone();
                        self.add_parents(commit, &commit_oid, &flags_clone)?;
                    }
                }
            } else {
                break;
            }
        }

        // Collect results (common ancestors that aren't stale)
        let mut result = Vec::new();
        for oid in &self.results {
            if !self.is_marked(oid, &Flag::Stale) {
                result.push(oid.clone());
            }
        }

        Ok(result)
    }

    pub fn is_marked(&self, oid: &str, flag: &Flag) -> bool {
        if let Some(flags) = self.flags.get(oid) {
            flags.contains(flag)
        } else {
            false
        }
    }

    fn all_stale(&self) -> bool {
        // Check if all commits in queue are marked as stale
        for oid in &self.queue {
            if !self.is_marked(oid, &Flag::Stale) {
                return false;
            }
        }
        true
    }

    fn add_parents(
        &mut self,
        commit: &Commit,
        commit_oid: &str,
        flags: &HashSet<Flag>,
    ) -> Result<(), Error> {
        // If commit has a parent, add it to the queue with the same flags
        if let Some(parent_oid) = commit.get_parent() {
            // Get or create flags entry for parent
            let current_flags = self.flags.entry(parent_oid.to_string()).or_insert_with(HashSet::new);
            
            // Skip if parent already has all these flags
            let mut new_flags_added = false;
            for flag in flags {
                if !current_flags.contains(flag) {
                    current_flags.insert(flag.clone());
                    new_flags_added = true;
                }
            }
            
            // Only add to queue if we added new flags
            if new_flags_added {
                self.queue.push_back(parent_oid.to_string());
            }
        }
        Ok(())
    }
}