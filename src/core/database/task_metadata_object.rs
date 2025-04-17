use std::any::Any;
use crate::core::database::database::GitObject;
use crate::core::commit_metadata::{TaskMetadata, TaskStatus};

pub struct TaskMetadataObject {
    oid: Option<String>,
    metadata: TaskMetadata,
}

impl TaskMetadataObject {
    pub fn new(metadata: TaskMetadata) -> Self {
        TaskMetadataObject {
            oid: None,
            metadata,
        }
    }
    
    pub fn get_metadata(&self) -> &TaskMetadata {
        &self.metadata
    }
}

impl GitObject for TaskMetadataObject {
    fn get_type(&self) -> &str {
        "task-meta"
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
        Box::new(TaskMetadataObject {
            oid: self.oid.clone(),
            metadata: self.metadata.clone(),
        })
    }
} 