// src/core/index/entry.rs
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::file_mode::FileMode;
const MAX_PATH_SIZE: u16 = 0xfff;

#[derive(Debug, Clone)]
pub struct Entry {
    // Existing fields...
    pub ctime: u32,
    pub ctime_nsec: u32,
    pub mtime: u32,
    pub mtime_nsec: u32,
    pub dev: u32,
    pub ino: u32,
    pub mode: FileMode,
    pub uid: u32,
    pub gid: u32,
    pub size: u32,
    pub oid: String,
    pub flags: u16,
    pub path: String,
    // Add this field:
    pub stage: u8,  // 0 = normal, 1 = base, 2 = ours, 3 = theirs
}

impl Entry {
    pub fn create(pathname: &Path, oid: &str, stat: &fs::Metadata) -> Self {
        let path = pathname.to_string_lossy().to_string();
        
        // Determine if file is executable (mode 755) or regular (mode 644)
        let mode = FileMode::from_metadata(stat);
        
        #[cfg(not(unix))]
        let mode = FileMode::REGULAR;
        
        let flags = path.len().min(MAX_PATH_SIZE as usize) as u16;
        
        // Get ctime and mtime
        let ctime = stat.created().unwrap_or(SystemTime::now());
        let mtime = stat.modified().unwrap_or(SystemTime::now());
        
        let ctime_duration = ctime.duration_since(UNIX_EPOCH).unwrap_or_default();
        let mtime_duration = mtime.duration_since(UNIX_EPOCH).unwrap_or_default();
        
        Entry {
            ctime: ctime_duration.as_secs() as u32,
            ctime_nsec: ctime_duration.subsec_nanos(),
            mtime: mtime_duration.as_secs() as u32,
            mtime_nsec: mtime_duration.subsec_nanos(),
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            size: stat.len() as u32,
            oid: oid.to_string(),
            flags,
            path,
            stage: 0,  // Default stage is 0 (normal entry)
        }
    }
    pub fn mode_octal(&self) -> String {
        self.mode.to_octal_string()
    }
    
    // Getteri pentru toate proprietățile
    pub fn get_ctime(&self) -> u32 {
        self.ctime
    }

    pub fn get_ctime_nsec(&self) -> u32 {
        self.ctime_nsec
    }

    pub fn get_mtime(&self) -> u32 {
        self.mtime
    }

    pub fn get_mtime_nsec(&self) -> u32 {
        self.mtime_nsec
    }

    pub fn get_dev(&self) -> u32 {
        self.dev
    }

    pub fn get_ino(&self) -> u32 {
        self.ino
    }

    pub fn get_mode(&self) -> &FileMode {
        &self.mode
    }

    pub fn get_uid(&self) -> u32 {
        self.uid
    }

    pub fn get_gid(&self) -> u32 {
        self.gid
    }

    pub fn get_size(&self) -> u32 {
        self.size
    }

    pub fn get_oid(&self) -> &str {
        &self.oid
    }

    pub fn get_flags(&self) -> u16 {
        self.flags
    }

    pub fn get_path(&self) -> &str {
        &self.path
    }

    // Setteri pentru proprietățile care ar putea necesita actualizare
    pub fn set_ctime(&mut self, ctime: u32) {
        self.ctime = ctime;
    }

    pub fn set_ctime_nsec(&mut self, ctime_nsec: u32) {
        self.ctime_nsec = ctime_nsec;
    }

    pub fn set_mtime(&mut self, mtime: u32) {
        self.mtime = mtime;
    }

    pub fn set_mtime_nsec(&mut self, mtime_nsec: u32) {
        self.mtime_nsec = mtime_nsec;
    }

    pub fn set_mode(&mut self, mode: FileMode) {
        self.mode = mode;
    }

    pub fn set_size(&mut self, size: u32) {
        self.size = size;
    }

    pub fn set_oid(&mut self, oid: String) {
        self.oid = oid;
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::new();
        
        // Pack all the fixed-size fields
        result.extend_from_slice(&self.ctime.to_be_bytes());
        result.extend_from_slice(&self.ctime_nsec.to_be_bytes());
        result.extend_from_slice(&self.mtime.to_be_bytes());
        result.extend_from_slice(&self.mtime_nsec.to_be_bytes());
        result.extend_from_slice(&self.dev.to_be_bytes());
        result.extend_from_slice(&self.ino.to_be_bytes());
        result.extend_from_slice(&self.mode.0.to_be_bytes());
        result.extend_from_slice(&self.uid.to_be_bytes());
        result.extend_from_slice(&self.gid.to_be_bytes());
        result.extend_from_slice(&self.size.to_be_bytes());
        
        // Convert OID from hex to binary (20 bytes)
        if let Ok(oid_bytes) = hex::decode(&self.oid) {
            result.extend_from_slice(&oid_bytes);
        } else {
            // If we cannot decode, just fill with zeros
            result.extend_from_slice(&[0; 20]);
        }
        
        // Add flags with stage bits
        // Stage is stored in the high bits of the flags field
        let flags_with_stage = self.flags | ((self.stage as u16) << 12);
        result.extend_from_slice(&flags_with_stage.to_be_bytes());
        
        // Add path
        result.extend_from_slice(self.path.as_bytes());
        result.push(0); // Null terminator
        
        // Pad to 8-byte boundary
        while result.len() % 8 != 0 {
            result.push(0);
        }
        
        result
    }
    
    
    pub fn parse(data: &[u8]) -> Result<Self, crate::errors::error::Error> {
        if data.len() < 62 {  // Minimum size without path
            return Err(crate::errors::error::Error::Generic("Entry data too short".to_string()));
        }
        
        // Parse all the fixed-size fields
        let ctime = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let ctime_nsec = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let mtime = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let mtime_nsec = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);
        let dev = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let ino = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        let mode_u32 = u32::from_be_bytes([data[24], data[25], data[26], data[27]]);
        let mode = FileMode(mode_u32);
        let uid = u32::from_be_bytes([data[28], data[29], data[30], data[31]]);
        let gid = u32::from_be_bytes([data[32], data[33], data[34], data[35]]);
        let size = u32::from_be_bytes([data[36], data[37], data[38], data[39]]);
        
        // Object ID is 20 bytes (40 hex chars)
        let oid = hex::encode(&data[40..60]);
        
        // Flags are 2 bytes
        let flags_with_stage = u16::from_be_bytes([data[60], data[61]]);
        let flags = flags_with_stage & 0x0FFF; // Lower 12 bits
        let stage = ((flags_with_stage >> 12) & 0x3) as u8; // Upper 2 bits (stage 0-3)
        
        // Path starts at byte 62 and continues until null byte
        let mut path_end = 62;
        while path_end < data.len() && data[path_end] != 0 {
            path_end += 1;
        }
        
        if path_end == data.len() {
            return Err(crate::errors::error::Error::Generic("No null terminator for path".to_string()));
        }
        
        let path = match std::str::from_utf8(&data[62..path_end]) {
            Ok(s) => s.to_string(),
            Err(_) => return Err(crate::errors::error::Error::Generic("Invalid UTF-8 in path".to_string())),
        };
        
        Ok(Entry {
            ctime,
            ctime_nsec,
            mtime,
            mtime_nsec,
            dev,
            ino,
            mode,
            uid,
            gid,
            size,
            oid,
            flags,
            path,
            stage,
        })
    }
    
    // Update stat information for an entry
    pub fn update_stat(&mut self, stat: &std::fs::Metadata) {
        // Update timestamps
        if let Ok(mtime) = stat.modified() {
            if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                self.set_mtime(duration.as_secs() as u32);
                self.set_mtime_nsec(duration.subsec_nanos());
            }
        }
        
        if let Ok(ctime) = stat.created() {
            if let Ok(duration) = ctime.duration_since(std::time::UNIX_EPOCH) {
                self.set_ctime(duration.as_secs() as u32);
                self.set_ctime_nsec(duration.subsec_nanos());
            }
        }
        
        // Update size
        self.set_size(stat.len() as u32);
        
        // Update mode using the new FileMode struct
        self.set_mode(FileMode::from_metadata(stat));
    }

    pub fn mode_match(&self, stat: &std::fs::Metadata) -> bool {
        let file_mode = FileMode::from_metadata(stat);
        FileMode::are_equivalent(self.mode.0, file_mode.0)
    }
    
    // Check if file timestamps match the entry's timestamps
    pub fn time_match(&self, stat: &std::fs::Metadata) -> bool {
        // We'll be more lenient with timestamp comparisons to avoid false positives
        // This is just an optimization, as we always check content hashes anyway
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            
            // Convert to seconds and nanoseconds for comparison
            let stat_mtime_sec = stat.mtime() as u32;
            
            // Instead of comparing nanoseconds, just check seconds
            // This avoids issues with filesystem timestamp precision
            self.get_mtime() == stat_mtime_sec
        }
        
        #[cfg(not(unix))]
        {
            // On Windows, we don't have the same granularity, so convert to seconds
            if let Ok(mtime) = stat.modified() {
                if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                    let stat_mtime_sec = duration.as_secs() as u32;
                    return self.get_mtime() == stat_mtime_sec;
                }
            }
            
            // If we can't get the modification time, assume they don't match
            false
        }
    }
    
    /// Check if a file is executable
    fn is_executable(&self, stat: &std::fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            return stat.permissions().mode() & 0o111 != 0;
        }
        
        #[cfg(not(unix))]
        {
            // Windows doesn't have simple executable bit
            // Just check if file has .exe, .bat, etc. extension
            let path = Path::new(&self.path);
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                return ext_str == "exe" || ext_str == "bat" || ext_str == "cmd";
            }
            false
        }
    }
    
}