// src/core/merge/bases.rs
use std::collections::HashSet; // Adaugă HashMap dacă nu există deja

use crate::core::database::database::Database;
use crate::errors::error::Error;
use crate::core::merge::common_ancestors::CommonAncestors;

pub struct Bases<'a> {
    database: &'a mut Database,
    commits: Vec<String>,
    redundant: HashSet<String>,
}

impl<'a> Bases<'a> {
    // Constructorul rămâne neschimbat (așteaptă doar database)
    pub fn new(database: &'a mut Database) -> Result<Self, Error> {
        Ok(Self {
            database,
            commits: Vec::new(),
            redundant: HashSet::new(),
        })
    }

    // Metoda find primește one și two ca argumente
    pub fn find(&mut self, one: &str, two: &str) -> Result<Vec<String>, Error> {
        let mut common = CommonAncestors::new(self.database, one, &[two])?;
        let initial_bases = common.find()?;

        // --- FIX: Deduplicare baze inițiale ---
        let unique_bases: HashSet<String> = initial_bases.into_iter().collect();
        self.commits = unique_bases.into_iter().collect(); // Stochează bazele unice
        // --- Sfârșit FIX ---

        if self.commits.len() <= 1 {
            return Ok(self.commits.clone());
        }

        self.redundant = HashSet::new();
        let commits_to_filter = self.commits.clone();
        for commit in commits_to_filter {
            // Pasează one și two la filter_commit
            self.filter_commit(&commit, one, two)?;
        }

        // Returnează doar bazele unice și non-redundante
        // (Filtrul ar trebui să mențină unicitatea, dar colectăm iar în HashSet pentru siguranță)
        let final_bases: HashSet<String> = self.commits.iter()
            .filter(|commit| !self.redundant.contains(*commit))
            .cloned()
            .collect();

        Ok(final_bases.into_iter().collect()) // Converteste înapoi în Vec
    }

    // filter_commit primește acum one și two
    fn filter_commit(&mut self, commit: &str, one: &str, two: &str) -> Result<(), Error> {
        if self.redundant.contains(commit) {
            return Ok(());
        }

        let others: Vec<_> = self.commits.iter()
            .filter(|oid| *oid != commit && !self.redundant.contains(*oid))
            .map(|oid| oid.as_str())
            .collect();

        if others.is_empty() {
            return Ok(());
        }

        // Găsește strămoșii comuni între commit și ceilalți, folosind one și two
        // Acest apel pare greșit - ar trebui să verificăm direct relația părinte-copil
        // Folosind CommonAncestors între `commit` și `other_oid`

        // Verificăm dacă `commit` este strămoș pentru oricare `other_oid`
        for other_oid in &others {
             let mut is_ancestor_check = CommonAncestors::new(self.database, commit, &[*other_oid])?;
             // common.find() returnează strămoșii comuni. Dacă `commit` e strămoș al lui `other_oid`,
             // atunci `find` ar trebui să returneze commit-uri mai vechi decât `commit`.
             // Verificarea corectă este dacă `other_oid` este accesibil din `commit` și nu invers.
             // Vom folosi o verificare mai directă: este `commit` un strămoș al lui `other_oid`?
             // Git folosește `git merge-base --is-ancestor commit other_oid`
             // Simulăm asta:
             if is_ancestor_check.find()?.contains(&commit.to_string()) { // Verifică dacă commit e printre strămoși
                // Dacă `commit` este strămoș al lui `other_oid`, atunci `commit` este redundant
                self.redundant.insert(commit.to_string());
                 println!("DEBUG Bases: Marking {} as redundant (ancestor of {})", commit, other_oid);
                // Putem ieși devreme dacă l-am marcat deja
                 return Ok(());
             }
        }

         // Verificăm dacă oricare `other_oid` este strămoș al lui `commit`
         for other_oid_str in others { // Iterăm prin others ca &str
             let mut is_descendant_check = CommonAncestors::new(self.database, other_oid_str, &[commit])?;
             // Verificăm dacă `other_oid_str` este strămoș al lui `commit`
             if is_descendant_check.find()?.contains(&other_oid_str.to_string()) {
                 // Dacă `other_oid_str` este strămoș al lui `commit`, atunci `other_oid_str` este redundant
                 self.redundant.insert(other_oid_str.to_string());
                  println!("DEBUG Bases: Marking {} as redundant (ancestor of {})", other_oid_str, commit);
             }
         }

        Ok(())
    }
}