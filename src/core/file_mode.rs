use std::fmt;

#[derive(PartialEq, Clone, Copy)] 
pub struct FileMode(pub u32);

impl FileMode {
    /// Mod pentru symlink-uri
    pub const SYMLINK: u32 = 0o120000;

    pub const REGULAR: FileMode = FileMode(0o100644);
    pub const EXECUTABLE: FileMode = FileMode(0o100755);
    pub const DIRECTORY: FileMode = FileMode(0o040000);
    
    /// Convertește un mod numeric la reprezentarea sa octală
    pub fn to_octal_string(&self) -> String {
        format!("{:o}", self.0)
    }
    
    /// Verifică dacă două moduri sunt echivalente, indiferent de reprezentare
    pub fn are_equivalent(mode1: u32, mode2: u32) -> bool {
        // Comparăm doar biții relevanți (permisiunile și tipul)
        // În mod normal, biții 12-15 (tipul) și 0-8 (permisiunile) sunt cei relevanți
        let mask = 0o170000 | 0o777; // Combină masca pentru tip și permisiuni
        (mode1 & mask) == (mode2 & mask)
    }
    
    /// Verifică dacă un mod corespunde unui fișier executabil
    pub fn is_executable(mode: u32) -> bool {
        (mode & 0o111) != 0
    }
    
    /// Determină modul corespunzător din metadatele unui fișier
    pub fn from_metadata(metadata: &std::fs::Metadata) -> FileMode {
        if metadata.is_dir() {
            return FileMode::DIRECTORY;
        }
    
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 != 0 {
                return FileMode::EXECUTABLE;
            }
        }
        
        FileMode::REGULAR
    }

    pub fn parse(mode_str: &str) -> Self {
        // Parsează ca număr octal
        let mode = u32::from_str_radix(mode_str, 8).unwrap_or(Self::REGULAR.0);
        FileMode(mode)
    }

    pub fn is_directory(&self) -> bool {
        *self == FileMode::DIRECTORY
    }
    
    // Add a static version of the method that takes a FileMode value
    pub fn is_directory_mode(mode: FileMode) -> bool {
        mode == FileMode::DIRECTORY
    }
}

impl fmt::Display for FileMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Afișează mereu în format octal
        write!(f, "{:o}", self.0)
    }
}

impl fmt::Debug for FileMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug afișează în octal și o notă clară
        write!(f, "FileMode({:o})", self.0)
    }
}