// Place this in an appropriate file (e.g., src/core/metadata.rs or inline where needed)

use std::fs::Metadata;
use std::time::SystemTime;

// Extension trait to add default() to Metadata
pub trait MetadataExt {
    fn default() -> Self;
}

// We can't implement Default directly on Metadata because it's a foreign type,
// but we can create a new struct that wraps it and implement conversions
#[derive(Debug, Clone)]
pub struct DefaultMetadata {
    size: u64,
    modified: SystemTime,
    created: SystemTime,
    is_dir: bool,
    is_file: bool,
}

impl DefaultMetadata {
    pub fn new() -> Self {
        DefaultMetadata {
            size: 0,
            modified: SystemTime::now(),
            created: SystemTime::now(),
            is_dir: false,
            is_file: true,
        }
    }
    
    pub fn to_metadata(&self) -> std::io::Result<Metadata> {
        // This is a placeholder - we can't actually create a Metadata directly
        // Instead, we'll use this in our code where a Metadata is needed
        unimplemented!("Cannot convert DefaultMetadata to Metadata directly")
    }
}

impl Default for DefaultMetadata {
    fn default() -> Self {
        Self::new()
    }
}

// Helper function to get default metadata for index operations
pub fn default_metadata() -> DefaultMetadata {
    DefaultMetadata::default()
}