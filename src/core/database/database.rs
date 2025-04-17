// src/core/database/database.rs
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::io::Read;
use std::collections::HashMap;
use sha1::{Digest, Sha1};
use flate2::write::ZlibEncoder;
use flate2::read::ZlibDecoder;
use flate2::Compression;
use crate::core::path_filter::PathFilter;
use crate::errors::error::Error;
use crate::core::database::blob::Blob;
use crate::core::database::tree::Tree;
use crate::core::database::commit::Commit;
use std::any::Any;

use super::entry::DatabaseEntry;
use super::tree_diff::TreeDiff;

pub struct Database {
    pub pathname: PathBuf,
    temp_chars: Vec<char>,
    objects: HashMap<String, Box<dyn GitObject>>,
}

impl Clone for Database {
    fn clone(&self) -> Self {
        Database {
            pathname: self.pathname.clone(),
            temp_chars: self.temp_chars.clone(),
            objects: HashMap::new(), // We don't clone the objects cache
        }
    }
}

pub trait GitObject: Any {
    fn get_type(&self) -> &str;
    fn to_bytes(&self) -> Vec<u8>;
    fn set_oid(&mut self, oid: String);
    fn as_any(&self) -> &dyn Any;
    fn clone_box(&self) -> Box<dyn GitObject>;
}

impl Database {
    pub fn new(pathname: PathBuf) -> Self {
        let temp_chars: Vec<char> = ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .collect();

        Database {
            pathname,
            temp_chars,
            objects: HashMap::new(),
        }
    }

    pub fn exists(&self, oid: &str) -> bool {
        self.object_path(oid).exists()
    }

    /// Încarcă un obiect din baza de date folosind OID-ul său
    pub fn load(&mut self, oid: &str) -> Result<Box<dyn GitObject>, Error> {
        // Verifică dacă obiectul e deja în cache
        if let Some(obj) = self.objects.get(oid) {
            // Clone the object using clone_box instead of direct clone
            return Ok(obj.clone_box());
        }

        // Citește obiectul și pune-l în cache
        let object = self.read_object(oid)?;
        let result = object.clone_box();
        self.objects.insert(oid.to_string(), object);
        
        Ok(result)
    }

    /// Metodă privată de clonare a unui obiect - implementare de bază
    fn clone_object(&self, obj: &Box<dyn GitObject>) -> Box<dyn GitObject> {
        // Use the new clone_box method instead of manual cloning
        obj.clone_box()
    }

    /// Stochează un obiect git în baza de date
    pub fn store(&mut self, object: &mut impl GitObject) -> Result<String, Error> {
        println!("Storing object of type: {}", object.get_type());
        
        // Serialize object
        let content = self.serialize_object(object)?;
        
        // Calculate OID (hash)
        let oid = self.hash_content(&content);
        println!("Calculated OID: {}", oid);
        
        // Write only if object doesn't already exist
        if !self.exists(&oid) {
            println!("Object {} doesn't exist, writing to database", oid);
            self.write_object(&oid, &content)?;
        } else {
            println!("Object {} already exists in database", oid);
        }
    
        // Set OID on object
        object.set_oid(oid.clone());
        
        // Verifică că OID-ul a fost setat corect
        if object.get_type() == "tree" {
            // Pentru verificare suplimentară la arbori (dacă implementați metoda get_oid())
            if let Some(tree) = object.as_any().downcast_ref::<Tree>() {
                if tree.get_oid().is_none() {
                    println!("WARNING: OID not set correctly on tree after store()!");
                }
            }
        }
    
        Ok(oid)
    }

    pub fn serialize_object(&self, object: &impl GitObject) -> Result<Vec<u8>, Error> {
        let obj_type = object.get_type();
        let content = object.to_bytes();
        println!("Serializing {} object, content size: {} bytes", obj_type, content.len());
        
        // Format: "<type> <size>\0<content>"
        let header = format!("{} {}\0", obj_type, content.len());
        let mut full_content = header.as_bytes().to_vec();
        full_content.extend_from_slice(&content);
        
        Ok(full_content)
    }

    /// Calculează hash-ul SHA-1 al conținutului
    pub fn hash_content(&self, content: &[u8]) -> String {
        let mut hasher = Sha1::new();
        hasher.update(content);
        let result = hasher.finalize();
        format!("{:x}", result)
    }

    /// Scrie un obiect în baza de date
    fn write_object(&self, oid: &str, content: &[u8]) -> Result<(), Error> {
        let object_path = self.object_path(oid);
        
        // Ieși devreme dacă obiectul există
        if object_path.exists() {
            return Ok(());
        }
        
        let dirname = object_path.parent().ok_or_else(|| {
            Error::Generic(format!("Invalid object path: {}", object_path.display()))
        })?;

        if !dirname.exists() {
            fs::create_dir_all(dirname)?;
        }

        let temp_name = self.generate_temp_name();
        let temp_path = dirname.join(temp_name);

        let mut file = File::create(&temp_path)?;

        // Comprimă și scrie
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(content)?;
        let compressed = encoder.finish()?;

        file.write_all(&compressed)?;
        fs::rename(temp_path, object_path)?;

        Ok(())
    }

    /// Obține calea către un obiect bazat pe OID
    fn object_path(&self, oid: &str) -> PathBuf {
        self.pathname.join(&oid[0..2]).join(&oid[2..])
    }

    /// Citește un obiect din baza de date și îl parsează
    /// Read and parse an object from the database
    fn read_object(&self, oid: &str) -> Result<Box<dyn GitObject>, Error> {
        let path = self.object_path(oid);
        
        if !path.exists() {
            return Err(Error::Generic(format!("Object not found: {}", oid)));
        }
        
        // Read the file
        let mut file = File::open(&path)?;
        let mut compressed_data = Vec::new();
        file.read_to_end(&mut compressed_data)?;
        
        // Decompress data
        let mut decoder = ZlibDecoder::new(&compressed_data[..]);
        let mut data = Vec::new();
        decoder.read_to_end(&mut data)?;
        
        // Parse header
        let null_pos = data.iter().position(|&b| b == 0)
            .ok_or_else(|| Error::Generic("Invalid object format: missing null byte".to_string()))?;
        
        let header = std::str::from_utf8(&data[0..null_pos])
            .map_err(|_| Error::Generic("Invalid header encoding".to_string()))?;
        
        let parts: Vec<&str> = header.split(' ').collect();
        if parts.len() != 2 {
            return Err(Error::Generic(format!("Invalid header format: {}", header)));
        }
        
        let obj_type = parts[0];
        let obj_size: usize = parts[1].parse()
            .map_err(|_| Error::Generic(format!("Invalid size in header: {}", parts[1])))?;
        
        // Verify size
        if obj_size != data.len() - null_pos - 1 {
            println!("Warning: Size mismatch in object {}: header claims {} bytes, actual content is {} bytes",
                oid, obj_size, data.len() - null_pos - 1);
        }
        
        // Extract content (after null byte)
        let content = &data[null_pos + 1..];
        
        // Parse object based on type
        let mut object: Box<dyn GitObject> = match obj_type {
        "blob" => {
            // Verifică dacă acest blob ar putea fi de fapt un director
            if content.len() >= 20 && (content[0] == b'4' && content[1] == b'0' && content[2] == b'0' && content[3] == b'0' && content[4] == b'0') {
                // Ar putea fi un arbore
                match Tree::parse(content) {
                    Ok(tree) => Box::new(tree),
                    Err(_) => Box::new(Blob::parse(content))
                }
            } else {
                Box::new(Blob::parse(content))
            }
        },
        "tree" => {
            println!("Parsing tree object: {}", oid);
            match Tree::parse(content) {
                Ok(tree) => Box::new(tree),
                Err(e) => {
                    println!("Error parsing tree {}: {}", oid, e);
                    return Err(e);
                }
            }
        },
            "commit" => match Commit::parse(content) {
                Ok(commit) => Box::new(commit),
                Err(e) => return Err(e),
            },
            "sprint-meta" => {
                // Parse the metadata from the encoded string
                let encoded = String::from_utf8_lossy(content).to_string();
                if let Some(metadata) = crate::core::branch_metadata::SprintMetadata::decode(&encoded) {
                    Box::new(crate::core::database::sprint_metadata_object::SprintMetadataObject::new(metadata))
                } else {
                    return Err(Error::Generic(format!("Invalid sprint metadata content: {}", encoded)))
                }
            },
            "task-meta" => {
                // Parse the metadata from the encoded string
                let encoded = String::from_utf8_lossy(content).to_string();
                if let Some(metadata) = crate::core::commit_metadata::TaskMetadata::decode(&encoded) {
                    Box::new(crate::core::database::task_metadata_object::TaskMetadataObject::new(metadata))
                } else {
                    return Err(Error::Generic(format!("Invalid task metadata content: {}", encoded)))
                }
            },
            _ => return Err(Error::Generic(format!("Unknown object type: {}", obj_type))),
        };
        
        // Set the OID
        object.set_oid(oid.to_string());
        
        Ok(object)
    }

    fn generate_temp_name(&self) -> String {
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        let name: String = (0..6)
            .map(|_| self.temp_chars.choose(&mut rng).unwrap())
            .collect();
        format!("tmp_obj_{}", name)
    }
    
    /// Helper method to calculate hash for raw data (useful for status command)
    pub fn hash_file_data(&self, data: &[u8]) -> String {
        let header = format!("blob {}\0", data.len());
        let mut full_content = header.as_bytes().to_vec();
        full_content.extend(data);
        
        self.hash_content(&full_content)
    }

    pub fn prefix_match(&self, prefix: &str) -> Result<Vec<String>, Error> {
        // Validate prefix is a valid hex string
        if !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(Vec::new());
        }
        
        // Ensure prefix is at least 2 characters (for directory)
        if prefix.len() < 2 {
            return Ok(Vec::new());
        }
        
        // Get the directory path for this prefix
        let dir_name = &prefix[0..2];
        let dir_path = self.pathname.join(dir_name);
        
        if !dir_path.exists() || !dir_path.is_dir() {
            return Ok(Vec::new());
        }
        
        // Read all files in the directory
        let entries = std::fs::read_dir(&dir_path).map_err(|e| Error::IO(e))?;
        
        // Filter files that match our prefix
        let mut matches = Vec::new();
        for entry_result in entries {
            match entry_result {
                Ok(entry) => {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    let full_id = format!("{}{}", dir_name, file_name);
                    
                    // Check if this ID starts with our prefix
                    if full_id.starts_with(prefix) {
                        matches.push(full_id);
                    }
                },
                Err(_) => continue,
            }
        }
        
        Ok(matches)
    }
    
    /// Get a short representation of an object ID
    pub fn short_oid(&self, oid: &str) -> String {
        if oid.len() <= 7 {
            oid.to_string()
        } else {
            oid[0..7].to_string()
        }
    }

    pub fn tree_diff(&mut self, a: Option<&str>, b: Option<&str>, filter: &PathFilter) -> Result<HashMap<PathBuf, (Option<DatabaseEntry>, Option<DatabaseEntry>)>, Error> {
        let mut diff = TreeDiff::new(self);
        diff.compare_oids(a, b, filter)?;
        Ok(diff.changes)
    }

    /// Obține un OID complet din unul prescurtat sau parțial
    pub fn resolve_oid(&self, partial_oid: &str) -> Result<String, Error> {
        // Dacă OID-ul are lungimea completă (40 de caractere), îl returnăm direct
        if partial_oid.len() == 40 && partial_oid.chars().all(|c| c.is_ascii_hexdigit()) {
            // Verificăm dacă obiectul există
            if self.exists(partial_oid) {
                return Ok(partial_oid.to_string());
            }
        }
        
        // Dacă OID-ul este parțial (minim 4 caractere), căutăm potriviri
        if partial_oid.len() >= 4 && partial_oid.chars().all(|c| c.is_ascii_hexdigit()) {
            let matches = self.prefix_match(partial_oid)?;
            
            if matches.is_empty() {
                return Err(Error::Generic(format!("No object found with prefix {}", partial_oid)));
            }
            
            if matches.len() > 1 {
                return Err(Error::Generic(format!("Ambiguous object prefix: {} matches multiple objects", partial_oid)));
            }
            
            return Ok(matches[0].clone());
        }
        
        Err(Error::Generic(format!("Invalid object identifier: {}", partial_oid)))
    }
}