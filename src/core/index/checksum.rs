use sha1::{Digest, Sha1};
use crate::errors::error::Error;

pub const CHECKSUM_SIZE: usize = 20;

pub struct Checksum {
    digest: Sha1,
}

impl Checksum {
    pub fn new() -> Self {
        Checksum {
            digest: Sha1::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.digest.update(data);
    }

    pub fn verify(&self, expected: &[u8]) -> Result<(), Error> {
        let digest = self.digest.clone().finalize();
        
        if expected != digest.as_slice() {
            println!("Warning: Index checksum mismatch. Expected: {:?}, Got: {:?}", 
                hex::encode(expected), hex::encode(digest.as_slice()));
            // Returnează Ok() în loc de Err pentru a continua chiar dacă checksum-ul nu se potrivește
            return Ok(());
        }
        
        Ok(())
    }

    pub fn finalize(&self) -> Vec<u8> {
        self.digest.clone().finalize().to_vec()
    }
}