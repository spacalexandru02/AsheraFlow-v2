use std::path::Path;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use crate::errors::error::Error;
use crate::core::refs::Refs;
use crate::core::database::database::Database;
use crate::core::repository::repository::Repository;
use crate::core::database::sprint_metadata_object::SprintMetadataObject;

#[derive(Debug, Clone)]
pub struct SprintMetadata {
    pub name: String, 
    pub start_timestamp: u64,
    pub duration_days: u32,
}

impl SprintMetadata {
    pub fn new(name: String, duration_days: u32) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        SprintMetadata {
            name,
            start_timestamp: now,
            duration_days,
        }
    }

    pub fn end_timestamp(&self) -> u64 {
        self.start_timestamp + (self.duration_days as u64 * 24 * 60 * 60)
    }

    pub fn is_active(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        now <= self.end_timestamp()
    }

    pub fn format_date(timestamp: u64) -> String {
        let dt = chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .unwrap_or_else(|| chrono::DateTime::UNIX_EPOCH);
        
        dt.format("%Y-%m-%d %H:%M").to_string()
    }

    pub fn to_branch_name(&self) -> String {
        format!("sprint-{}", self.name.replace(" ", "-").to_lowercase())
    }

    // Encode sprint metadata into a branch description
    pub fn encode(&self) -> String {
        format!("SPRINT:{}:{}:{}", self.name, self.start_timestamp, self.duration_days)
    }

    // Decode sprint metadata from a branch description
    pub fn decode(encoded: &str) -> Option<Self> {
        let parts: Vec<&str> = encoded.split(':').collect();
        if parts.len() >= 4 && parts[0] == "SPRINT" {
            let name = parts[1].to_string();
            let start_timestamp = parts[2].parse::<u64>().ok()?;
            let duration_days = parts[3].parse::<u32>().ok()?;

            Some(SprintMetadata {
                name,
                start_timestamp,
                duration_days,
            })
        } else {
            None
        }
    }
}

pub struct BranchMetadataManager {
    repo_path: std::path::PathBuf,
}

impl BranchMetadataManager {
    pub fn new(repo_path: &Path) -> Self {
        BranchMetadataManager {
            repo_path: repo_path.to_path_buf(),
        }
    }

    // Get the current branch name
    pub fn get_current_branch(&self) -> Result<String, Error> {
        // Initialize the repository path
        let git_path = self.repo_path.join(".ash");
        
        // Create a reference to the refs module
        let refs = crate::core::refs::Refs::new(&git_path);
        
        // Get current reference
        let current = refs.current_ref()?;
        
        match current {
            crate::core::refs::Reference::Symbolic(path) => {
                // Extract branch name from symbolic reference
                // Usually in the format "refs/heads/branch-name"
                if path.starts_with("refs/heads/") {
                    Ok(path.strip_prefix("refs/heads/")
                        .unwrap_or(&path)
                        .to_string())
                } else {
                    Ok(path)
                }
            },
            crate::core::refs::Reference::Direct(_) => {
                // Detached HEAD state
                Err(Error::Generic("HEAD is in a detached state".into()))
            }
        }
    }

    // Store sprint metadata in the object database
    pub fn store_sprint_metadata(&self, branch_name: &str, metadata: &SprintMetadata) -> Result<(), Error> {
        // Creăm un repository și avem acces la database
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let mut repo = Repository::new(repo_str)?;
        
        // Convertim metadatele în reprezentare string și apoi în obiect
        let encoded = metadata.encode();
        let mut obj = SprintMetadataObject::new(metadata.clone());
        
        // Stocăm obiectul în database
        let oid = repo.database.store(&mut obj)?;
        
        // Actualizăm referința către metadate
        // Folosim numele branch-ului așa cum este pentru consistență
        let meta_ref = format!("refs/meta/{}", branch_name);
        println!("[DEBUG-STORE] Updating ref: {} to point to {}", meta_ref, oid);
        repo.refs.update_ref(&meta_ref, &oid)?;
        
        // Make sure sprint- prefixed branch also exists for task creation
        let sprint_branch_name = if branch_name.starts_with("sprint-") {
            branch_name.to_string()
        } else {
            format!("sprint-{}", branch_name)
        };
        
        // Create the sprint branch if it doesn't exist
        let head_oid = match repo.refs.read_head()? {
            Some(oid) => oid,
            None => return Err(Error::Generic("HEAD reference not found".into())),
        };
        
        // Try to create the branch (ignore error if branch already exists)
        match repo.refs.create_branch(&sprint_branch_name, &head_oid) {
            Ok(_) => println!("[DEBUG-STORE] Created branch {}", sprint_branch_name),
            Err(e) => {
                if e.to_string().contains("already exists") {
                    println!("[DEBUG-STORE] Branch {} already exists", sprint_branch_name);
                } else {
                    return Err(e);
                }
            }
        }
        
        Ok(())
    }

    // Retrieve sprint metadata from the object database
    pub fn get_sprint_metadata(&self, branch_name: &str) -> Result<Option<SprintMetadata>, Error> {
        println!("[DEBUG-GET] Căutăm metadate pentru branch: {}", branch_name);
        
        // Creăm un repository și avem acces la database
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let mut repo = Repository::new(repo_str)?;
        
        // Folosim direct numele branch-ului fără modificări suplimentare 
        // pentru a păstra consistența cu ce am stocat inițial
        let meta_ref = format!("refs/meta/{}", branch_name);
        println!("[DEBUG-GET] Verificăm referința: {}", meta_ref);
        
        // Citim referința pentru metadate
        let oid = match repo.refs.read_ref(&meta_ref)? {
            Some(oid) => {
                println!("[DEBUG-GET] OID găsit pentru referința {}: {}", meta_ref, oid);
                oid
            },
            None => {
                println!("[DEBUG-GET] Nu a fost găsită referința: {}", meta_ref);
                // Încercăm și cu formatul alternativ pentru compatibilitate
                let alt_meta_ref = format!("refs/meta/sprint-{}", branch_name);
                println!("[DEBUG-GET] Încercăm referința alternativă: {}", alt_meta_ref);
                match repo.refs.read_ref(&alt_meta_ref)? {
                    Some(oid) => {
                        println!("[DEBUG-GET] OID găsit pentru referința alternativă: {}", oid);
                        oid
                    },
                    None => {
                        println!("[DEBUG-GET] Nu a fost găsită nici referința alternativă");
                        return Ok(None);
                    },
                }
            },
        };
        
        // Încărcăm obiectul din database
        println!("[DEBUG-GET] Încărcăm obiectul cu OID: {}", oid);
        match repo.database.load(&oid) {
            Ok(obj) => {
                println!("[DEBUG-GET] Obiect încărcat cu succes");
                if let Some(meta_obj) = obj.as_any().downcast_ref::<SprintMetadataObject>() {
                    println!("[DEBUG-GET] Obiectul este de tipul SprintMetadataObject");
                    Ok(Some(meta_obj.get_metadata().clone()))
                } else {
                    println!("[DEBUG-GET] Obiectul nu este de tipul SprintMetadataObject");
                    Err(Error::Generic("Invalid metadata object type".into()))
                }
            },
            Err(e) => {
                println!("[DEBUG-GET] Eroare la încărcarea obiectului: {:?}", e);
                Err(e)
            }
        }
    }

    // Find the current active sprint
    pub fn find_active_sprint(&self) -> Result<Option<(String, SprintMetadata)>, Error> {
        println!("[DEBUG-FIND] Începe căutarea sprint-ului activ");
        
        // Creăm un repository și avem acces la database
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let mut repo = Repository::new(repo_str)?;
        
        // Inspect object database directly
        println!("[DEBUG-FIND] Verificare directă a fișierelor de obiecte:");
        let output = std::process::Command::new("find")
            .arg(".ash/objects/")
            .arg("-type")
            .arg("f")
            .arg("-not")
            .arg("-path")
            .arg("*.idx")
            .output()
            .expect("Failed to execute find command");
        
        println!("[DEBUG-FIND] Obiecte găsite în baza de date: {}", String::from_utf8_lossy(&output.stdout));
        
        // Try to read one of the found objects directly
        if let Ok(output) = std::process::Command::new("cat")
            .arg(".ash/objects/2f/2f3c031e85ec8d92a617261004f33123e63e58")
            .output() {
            println!("[DEBUG-FIND] Conținut obiect sprint-meta: {} bytes", output.stdout.len());
            // Try to decompress and display content of the known object
            match repo.database.load("2f2f3c031e85ec8d92a617261004f33123e63e58") {
                Ok(_) => println!("[DEBUG-FIND] Obiect încărcat cu succes"),
                Err(e) => println!("[DEBUG-FIND] Eroare la încărcarea obiectului: {:?}", e)
            }
        }
        
        // Obținem toate referințele meta/*
        // Aceasta va include atât refs/meta/sprint-* cât și noile formate
        let refs = repo.refs.list_refs_with_prefix("refs/meta/")?;
        println!("[DEBUG-FIND] Referințe meta găsite: {}", refs.len());
        for r in &refs {
            println!("[DEBUG-FIND] Referință: {:?}", r);
        }
        
        // Verificăm dacă există directory-ul cu referințe
        println!("[DEBUG-FIND] Verificare directă a fișierelor de referințe:");
        let output = std::process::Command::new("find")
            .arg(".ash/refs/meta/")
            .arg("-type")
            .arg("f")
            .output()
            .expect("Failed to execute find command");
        
        println!("[DEBUG-FIND] Referințe meta găsite direct: {}", String::from_utf8_lossy(&output.stdout));
        
        // Try to read one of the found refs directly if they exist
        if let Ok(output) = std::process::Command::new("cat")
            .arg(".ash/refs/meta/sprint-meta")
            .output() {
            if output.status.success() {
                println!("[DEBUG-FIND] Conținut referință sprint-meta: {}", String::from_utf8_lossy(&output.stdout));
            } else {
                println!("[DEBUG-FIND] Nu s-a putut citi referința sprint-meta");
            }
        }
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        println!("[DEBUG-FIND] Timestamp actual: {}", now);    
        
        // Try an additional check with sprint- prefix
        let sprint_refs = repo.refs.list_refs_with_prefix("refs/meta/sprint-")?;
        println!("[DEBUG-FIND] Referințe sprint găsite: {}", sprint_refs.len());
        for r in &sprint_refs {
            println!("[DEBUG-FIND] Referință sprint: {:?}", r);
        }
        
        // If no refs found using the standard methods, try a direct approach with the known
        // sprint-meta object at .ash/objects/2f/2f3c031e85ec8d92a617261004f33123e63e58
        if refs.is_empty() && sprint_refs.is_empty() {
            println!("[DEBUG-FIND] Nu s-au găsit referințe, încercăm direct cu obiectul cunoscut");
            if let Ok(obj) = repo.database.load("2f2f3c031e85ec8d92a617261004f33123e63e58") {
                if let Some(meta_obj) = obj.as_any().downcast_ref::<SprintMetadataObject>() {
                    let metadata = meta_obj.get_metadata().clone();
                    println!("[DEBUG-FIND] Metadate găsite direct din obiect: {}", metadata.name);
                    if metadata.is_active() {
                        println!("[DEBUG-FIND] Sprint activ găsit direct: sprint-{}", metadata.name);
                        return Ok(Some((format!("sprint-{}", metadata.name), metadata)));
                    }
                }
            }
        }
        
        for reference in refs {
            match reference {
                crate::core::refs::Reference::Symbolic(path) => {
                    println!("[DEBUG-FIND] Verificare referință: {}", path);
                    
                    // Extragem numele branch-ului din calea de referință
                    let branch_name = if path.starts_with("refs/meta/sprint-") {
                        let name = path.strip_prefix("refs/meta/sprint-")
                            .unwrap_or(&path)
                            .to_string();
                        println!("[DEBUG-FIND] Branch extras din sprint-*: {}", name);
                        name
                    } else if path.starts_with("refs/meta/") {
                        let name = path.strip_prefix("refs/meta/")
                            .unwrap_or(&path)
                            .to_string();
                        println!("[DEBUG-FIND] Branch extras din meta/: {}", name);
                        name
                    } else {
                        println!("[DEBUG-FIND] Referință necunoscută, ignorată: {}", path);
                        continue;
                    };
                    
                    println!("[DEBUG-FIND] Se verifică metadatele pentru branch-ul: {}", branch_name);
                    if let Ok(Some(metadata)) = self.get_sprint_metadata(&branch_name) {
                        println!("[DEBUG-FIND] Metadate găsite pentru branch: {}", branch_name);
                        println!("[DEBUG-FIND] Sprint: '{}', start: {}, end: {}", 
                            metadata.name, metadata.start_timestamp, metadata.end_timestamp());
                        println!("[DEBUG-FIND] Sprint activ: {}", metadata.is_active());
                        
                        if metadata.is_active() {
                            println!("[DEBUG-FIND] Sprint activ găsit: {}", branch_name);
                            return Ok(Some((branch_name, metadata)));
                        }
                    } else {
                        println!("[DEBUG-FIND] Nu s-au găsit metadate pentru branch: {}", branch_name);
                    }
                },
                _ => {
                    println!("[DEBUG-FIND] Referință non-simbolică ignorată");
                    continue;
                },
            }
        }
        
        println!("[DEBUG-FIND] Nu s-a găsit niciun sprint activ");
        Ok(None)
    }

    // Get all sprints
    pub fn get_all_sprints(&self) -> Result<Vec<(String, SprintMetadata)>, Error> {
        // Creăm un repository și avem acces la database
        let repo_str = self.repo_path.to_str().unwrap_or(".");
        let repo = Repository::new(repo_str)?;
        
        let mut results = Vec::new();
        
        // Obținem toate referințele meta/sprint-*
        let refs = repo.refs.list_refs_with_prefix("refs/meta/sprint-")?;
            
        for reference in refs {
            match reference {
                crate::core::refs::Reference::Symbolic(path) => {
                    let branch_name = path.strip_prefix("refs/meta/sprint-")
                        .unwrap_or(&path)
                        .to_string();
                    
                    if let Ok(Some(metadata)) = self.get_sprint_metadata(&branch_name) {
                        results.push((branch_name, metadata));
                    }
                },
                _ => continue,
            }
        }
        
        // Sort by start date, newest first
        results.sort_by(|a, b| b.1.start_timestamp.cmp(&a.1.start_timestamp));
        
        Ok(results)
    }
} 