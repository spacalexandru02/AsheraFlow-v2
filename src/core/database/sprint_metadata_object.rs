use std::any::Any;
use crate::core::database::database::GitObject;
use crate::core::branch_metadata::SprintMetadata;

pub struct SprintMetadataObject {
    oid: Option<String>,
    metadata: SprintMetadata,
}

impl SprintMetadataObject {
    pub fn new(metadata: SprintMetadata) -> Self {
        SprintMetadataObject {
            oid: None,
            metadata,
        }
    }
    
    pub fn get_metadata(&self) -> &SprintMetadata {
        &self.metadata
    }
}

impl GitObject for SprintMetadataObject {
    fn get_type(&self) -> &str {
        "sprint-meta"
    }
    
    fn to_bytes(&self) -> Vec<u8> {
        self.metadata.encode().into_bytes()
    }
    
    fn set_oid(&mut self, oid: String) {
        self.oid = Some(oid);
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    fn clone_box(&self) -> Box<dyn GitObject> {
        Box::new(SprintMetadataObject {
            oid: self.oid.clone(),
            metadata: self.metadata.clone(),
        })
    }
} 