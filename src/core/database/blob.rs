// src/core/database/blob.rs with clone_box implementation
use super::database::GitObject;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct Blob {
    oid: Option<String>,
    data: Vec<u8>,
}

impl GitObject for Blob {
    fn get_type(&self) -> &str {
        "blob"
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.data.clone()
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

impl Blob {
    pub fn new(data: Vec<u8>) -> Self {
        Blob { oid: None, data }
    }

    pub fn get_oid(&self) -> Option<&String> {
        self.oid.as_ref()
    }
    
    /// Parsează un blob dintr-un șir de bytes
    pub fn parse(data: &[u8]) -> Self {
        Blob::new(data.to_vec())
    }
}