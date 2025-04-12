// src/core/merge/inputs.rs
use crate::errors::error::Error;
use crate::core::merge::bases::Bases;
use crate::core::database::database::Database;
use crate::core::refs::Refs;
// Eliminăm importul Revision dacă nu este folosit direct aici
// import crate::core::revision::Revision;

pub trait MergeInputs {
    fn left_name(&self) -> String;
    fn right_name(&self) -> String;
    fn left_oid(&self) -> String;
    fn right_oid(&self) -> String;
    fn base_oids(&self) -> Vec<String>;
}

#[derive(Debug)]
pub struct Inputs {
    pub left_name: String,
    pub right_name: String,
    pub left_oid: String,
    pub right_oid: String,
    pub base_oids: Vec<String>,
}

impl Inputs {
    pub fn new(
        database: &mut Database,
        refs: &Refs,
        left_name: String,
        right_name: String
    ) -> Result<Self, Error> {
        // Resolve the OIDs for the left and right revisions
        let left_oid = Self::resolve_rev(database, refs, &left_name)?;
        let right_oid = Self::resolve_rev(database, refs, &right_name)?;

        // Find the common base(s) between the two commits
        // --- FIX APPLIED HERE ---
        // 1. Call Bases::new with only the database argument
        let mut common = Bases::new(database)?;
        let base_oids = common.find(&left_oid, &right_oid)?;
        println!("DEBUG: Found base_oids: {:?}", base_oids); // <-- Adaugă aici
        println!("DEBUG: left_oid: {}", left_oid); // <-- Adaugă aici
        let is_ff = base_oids == vec![left_oid.clone()]; // <-- Verifică logica
        println!("DEBUG: is_fast_forward check result: {}", is_ff); 
        // 2. Call common.find with the left_oid and right_oid arguments
        let base_oids = common.find(&left_oid, &right_oid)?;

        Ok(Self {
            left_name,
            right_name,
            left_oid,
            right_oid,
            base_oids,
        })
    }

    pub fn already_merged(&self) -> bool {
        // Check if right is already fully merged into left
        self.base_oids == vec![self.right_oid.clone()]
    }

    pub fn is_fast_forward(&self) -> bool {
        // Check if left is an ancestor of right (fast-forward possible)
        self.base_oids == vec![self.left_oid.clone()]
    }

    // Această funcție probabil ar trebui să folosească structura Revision dacă
    // vrei să gestionezi sintaxe mai complexe gen "HEAD~1" etc.
    // Am păstrat implementarea simplă de mai devreme pentru moment.
    fn resolve_rev(database: &mut Database, refs: &Refs, rev: &str) -> Result<String, Error> {
        // First check if it's a direct ref (like HEAD)
        if let Ok(Some(oid)) = refs.read_ref(rev) {
            return Ok(oid);
        }

        // Next check if it's a branch name (e.g., "master")
        let branch_path = format!("refs/heads/{}", rev);
        if let Ok(Some(oid)) = refs.read_ref(&branch_path) {
            return Ok(oid);
        }

        // Last, check if it's a valid object ID (full or abbreviated)
        // Ar trebui să folosim Revision::resolve pentru a gestiona și OID-uri abreviate
        // Dar pentru moment, verificăm doar existența ca OID complet.
        if database.exists(rev) {
             // Ideal ar fi să verificăm dacă este un commit aici.
             // let obj = database.load(rev)?;
             // if obj.get_type() == "commit" {
                 return Ok(rev.to_string());
             // }
        }

        // If none of the above, try using Revision to resolve more complex cases
        // This requires passing the Repository struct, which isn't directly available here.
        // You might need to refactor how Repository/Database/Refs are passed around,
        // or pass the Repository itself to Inputs::new.
        // For now, we'll stick to the simpler resolution logic.

        // Could not resolve revision
        Err(Error::Generic(format!("Not a valid revision: '{}'", rev)))
    }
}

impl MergeInputs for Inputs {
    fn left_name(&self) -> String {
        self.left_name.clone()
    }

    fn right_name(&self) -> String {
        self.right_name.clone()
    }

    fn left_oid(&self) -> String {
        self.left_oid.clone()
    }

    fn right_oid(&self) -> String {
        self.right_oid.clone()
    }

    fn base_oids(&self) -> Vec<String> {
        self.base_oids.clone()
    }
}

// Implementation for cherry-pick (which is similar to merge but with different base assumptions)
#[derive(Debug)]
pub struct CherryPick {
    pub left_name: String,
    pub right_name: String,
    pub left_oid: String,
    pub right_oid: String,
    pub base_oids: Vec<String>,
}

impl CherryPick {
    pub fn new(
        left_name: String,
        right_name: String,
        left_oid: String,
        right_oid: String,
        base_oids: Vec<String>,
    ) -> Self {
        Self {
            left_name,
            right_name,
            left_oid,
            right_oid,
            base_oids,
        }
    }
}

impl MergeInputs for CherryPick {
    fn left_name(&self) -> String {
        self.left_name.clone()
    }

    fn right_name(&self) -> String {
        self.right_name.clone()
    }

    fn left_oid(&self) -> String {
        self.left_oid.clone()
    }

    fn right_oid(&self) -> String {
        self.right_oid.clone()
    }

    fn base_oids(&self) -> Vec<String> {
        self.base_oids.clone()
    }
}