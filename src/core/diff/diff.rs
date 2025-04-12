// src/core/diff/diff.rs - versiune completă îmbunătățită
use std::fs;
use std::path::Path;
use crate::core::workspace::Workspace;
use crate::core::database::database::{Database, GitObject};
use crate::core::database::blob::Blob;
use crate::errors::error::Error;
use crate::core::color::Color;
use super::myers;

/// Dimensiunea maximă a unui fișier pentru diff (pentru a evita probleme de performanță)
const MAX_DIFF_SIZE: usize = 10 * 1024 * 1024; // 10 MB

/// Împarte un șir în linii
pub fn split_lines(content: &str) -> Vec<String> {
    content.lines().map(|s| s.to_string()).collect()
}

/// Citește un fișier și împarte conținutul său în linii
pub fn read_file_lines(path: &Path) -> Result<Vec<String>, std::io::Error> {
    let content = fs::read_to_string(path)?;
    Ok(split_lines(&content))
}

/// Compară două fișiere și returnează un diff formatat
pub fn diff_files(file1_path: &Path, file2_path: &Path, context_lines: usize) -> Result<String, Error> {
    // Mai întâi, citim datele brute pentru a verifica dacă fișierele sunt binare
    let file1_data = fs::read(file1_path).map_err(|e| Error::IO(e))?;
    let file2_data = fs::read(file2_path).map_err(|e| Error::IO(e))?;
    
    // Verifică dacă fișierele sunt prea mari pentru diff
    if file1_data.len() > MAX_DIFF_SIZE || file2_data.len() > MAX_DIFF_SIZE {
        return Ok(format!("File too large for diff: maximum size is {} bytes", MAX_DIFF_SIZE));
    }
    
    // Verifică dacă fișierele sunt binare
    let file1_is_binary = myers::is_binary_content(&file1_data);
    let file2_is_binary = myers::is_binary_content(&file2_data);
    
    if file1_is_binary || file2_is_binary {
        return Ok(format!("Binary files {} and {} differ", 
                         file1_path.display(), file2_path.display()));
    }
    
    // Continuă cu diff-ul text normal
    let diff_content = match (String::from_utf8(file1_data.clone()), String::from_utf8(file2_data.clone())) {
        (Ok(content1), Ok(content2)) => {
            let a_lines = split_lines(&content1);
            let b_lines = split_lines(&content2);
            
            let edits = myers::diff_lines(&a_lines, &b_lines);
            myers::format_diff(&a_lines, &b_lines, &edits, context_lines)
        },
        _ => {
            // Fișierele nu sunt UTF-8 valid, dar au trecut verificarea binară
            // Le tratăm ca text non-UTF-8, folosind from_utf8_lossy
            let content1 = String::from_utf8_lossy(&file1_data);
            let content2 = String::from_utf8_lossy(&file2_data);
            
            let a_lines = split_lines(&content1);
            let b_lines = split_lines(&content2);
            
            let edits = myers::diff_lines(&a_lines, &b_lines);
            myers::format_diff(&a_lines, &b_lines, &edits, context_lines)
        }
    };
    
    // Adaugă antetul git-style
    let file1_name = file1_path.file_name().unwrap_or_default().to_string_lossy();
    let file2_name = file2_path.file_name().unwrap_or_default().to_string_lossy();
    
    // Calculează hash-uri fictive pentru a simula formatul git
    // Folosim primele și ultimele bytes pentru a genera hash-uri simple
    let hash1 = format!("{:08x}", file1_data.iter().fold(0u32, |acc, &b| acc.wrapping_add(b as u32)));
    let hash2 = format!("{:08x}", file2_data.iter().fold(0u32, |acc, &b| acc.wrapping_add(b as u32)));
    
    // Ia doar primele 7 caractere pentru a respecta formatul git
    let hash1_short = &hash1[0..std::cmp::min(7, hash1.len())];
    let hash2_short = &hash2[0..std::cmp::min(7, hash2.len())];
    
    // Creează antetul în stil git
    let mut result = String::new();
    result.push_str(&format!("index {}..{} 100644\n", hash1_short, hash2_short));
    result.push_str(&format!("--- a/{}\n", file1_name));
    result.push_str(&format!("+++ b/{}\n", file2_name));
    result.push_str(&diff_content);
    
    Ok(result)
}

/// Compară un fișier cu versiunea sa din baza de date
pub fn diff_with_database(
    workspace: &Workspace, 
    database: &mut Database,
    file_path: &Path, 
    oid: &str,
    context_lines: usize
) -> Result<String, Error> {
    // Citește copia de lucru
    let working_content = workspace.read_file(file_path)?;
    
    // Citește versiunea din baza de date
    let blob_obj = database.load(oid)?;
    let blob = match blob_obj.as_any().downcast_ref::<Blob>() {
        Some(b) => b,
        None => return Err(Error::Generic(format!("Object {} is not a blob", oid))),
    };
    
    let db_content = blob.to_bytes();
    
    // Verifică dacă conținutul este binar
    let working_is_binary = myers::is_binary_content(&working_content);
    let db_is_binary = myers::is_binary_content(&db_content);
    
    if working_is_binary || db_is_binary {
        return Ok(format!("Binary files differ"));
    }
    
    // Verifică dacă fișierele sunt prea mari pentru diff
    if working_content.len() > MAX_DIFF_SIZE || db_content.len() > MAX_DIFF_SIZE {
        return Ok(format!("File too large for diff: maximum size is {} bytes", MAX_DIFF_SIZE));
    }
    
    // Calculează hash-ul pentru conținutul fișierului de lucru
    let working_hash = database.hash_file_data(&working_content);
    
    // Convertește conținutul în text și calculează diff-ul
    let diff_content = match (String::from_utf8(working_content.to_vec()), String::from_utf8(db_content.to_vec())) {
        (Ok(working_text), Ok(db_text)) => {
            // Ambele sunt UTF-8 valid
            let working_lines = split_lines(&working_text);
            let db_lines = split_lines(&db_text);
            
            let edits = myers::diff_lines(&db_lines, &working_lines);
            myers::format_diff(&db_lines, &working_lines, &edits, context_lines)
        },
        _ => {
            // Cel puțin unul dintre fișiere nu este UTF-8 valid
            // Le tratăm ca text non-UTF-8, folosind from_utf8_lossy
            let working_text = String::from_utf8_lossy(&working_content);
            let db_text = String::from_utf8_lossy(&db_content);
            
            let working_lines = split_lines(&working_text);
            let db_lines = split_lines(&db_text);
            
            let edits = myers::diff_lines(&db_lines, &working_lines);
            myers::format_diff(&db_lines, &working_lines, &edits, context_lines)
        }
    };
    
    // Verifică dacă diff-ul este gol (fișierele sunt identice)
    if diff_content.trim().is_empty() {
        return Ok(format!("Files are identical"));
    }
    
    // Adaugă antetul git-style cu informații despre index
    let path_str = file_path.to_string_lossy();
    
    // Generează hash-uri scurte pentru a simula formatul git
    let db_hash_short = if oid.len() >= 7 { &oid[0..7] } else { oid };
    let working_hash_short = if working_hash.len() >= 7 { &working_hash[0..7] } else { &working_hash };
    
    // Creează antetul în stil git
    let mut result = String::new();
    result.push_str(&format!("index {}..{} 100644\n", db_hash_short, working_hash_short));
    result.push_str(&format!("--- a/{}\n", path_str));
    result.push_str(&format!("+++ b/{}\n", path_str));
    result.push_str(&diff_content);
    
    Ok(result)
}


/// Calculează și afișează diff-ul în mod incremental pentru fișiere mari
pub fn incremental_diff_with_database(
    workspace: &Workspace,
    database: &mut Database,
    file_path: &Path,
    oid: &str,
    context_lines: usize
) -> Result<String, Error> {
    // Citește copia de lucru
    let working_content = workspace.read_file(file_path)?;
    
    // Citește versiunea din baza de date
    let blob_obj = database.load(oid)?;
    let blob = match blob_obj.as_any().downcast_ref::<Blob>() {
        Some(b) => b,
        None => return Err(Error::Generic(format!("Object {} is not a blob", oid))),
    };
    
    let db_content = blob.to_bytes();
    
    // Verifică dacă conținutul este binar
    if myers::is_binary_content(&working_content) || myers::is_binary_content(&db_content) {
        return Ok(format!("Binary files differ"));
    }
    
    // Pentru fișiere mari, folosim o abordare incrementală, procesând porțiuni din fișier
    if working_content.len() > MAX_DIFF_SIZE || db_content.len() > MAX_DIFF_SIZE {
        // Împarte fișierele în secțiuni de 100kb
        const CHUNK_SIZE: usize = 100 * 1024;
        let mut diff_content = String::new();
        
        diff_content.push_str(&format!("Large file: showing first chunks only (file size: {} bytes)\n", 
                               std::cmp::max(working_content.len(), db_content.len())));
        
        // Procesează primul chunk
        let db_chunk = if db_content.len() > CHUNK_SIZE {
            &db_content[0..CHUNK_SIZE]
        } else {
            &db_content
        };
        
        let working_chunk = if working_content.len() > CHUNK_SIZE {
            &working_content[0..CHUNK_SIZE]
        } else {
            &working_content
        };
        
        // Calculează diff-ul pentru acest chunk
        let db_text = String::from_utf8_lossy(db_chunk);
        let working_text = String::from_utf8_lossy(working_chunk);
        
        let db_lines = split_lines(&db_text);
        let working_lines = split_lines(&working_text);
        
        let edits = myers::diff_lines(&db_lines, &working_lines);
        let chunk_diff = myers::format_diff(&db_lines, &working_lines, &edits, context_lines);
        
        diff_content.push_str(&chunk_diff);
        
        // Adaugă o notă că am arătat doar o parte din fișier
        if db_content.len() > CHUNK_SIZE || working_content.len() > CHUNK_SIZE {
            diff_content.push_str("\n...\n(Diff truncated, file too large)\n");
        }
        
        // Adaugă antetul git-style
        let path_str = file_path.to_string_lossy();
        
        // Calculează hash-ul pentru conținutul de lucru
        let working_hash = database.hash_file_data(&working_content);
        
        // Generează hash-uri scurte pentru a simula formatul git
        let db_hash_short = if oid.len() >= 7 { &oid[0..7] } else { oid };
        let working_hash_short = if working_hash.len() >= 7 { &working_hash[0..7] } else { &working_hash };
        
        // Creează antetul în stil git
        let mut result = String::new();
        result.push_str(&format!("index {}..{} 100644\n", db_hash_short, working_hash_short));
        result.push_str(&format!("--- a/{}\n", path_str));
        result.push_str(&format!("+++ b/{}\n", path_str));
        result.push_str(&diff_content);
        
        return Ok(result);
    }
    
    // Pentru fișiere de dimensiune normală, continuă cu diff-ul complet
    let diff_content = match (String::from_utf8(working_content.to_vec()), String::from_utf8(db_content.to_vec())) {
        (Ok(working_text), Ok(db_text)) => {
            // Ambele sunt UTF-8 valid
            let working_lines = split_lines(&working_text);
            let db_lines = split_lines(&db_text);
            
            let edits = myers::diff_lines(&db_lines, &working_lines);
            myers::format_diff(&db_lines, &working_lines, &edits, context_lines)
        },
        _ => {
            // Cel puțin unul dintre fișiere nu este UTF-8 valid
            let working_text = String::from_utf8_lossy(&working_content);
            let db_text = String::from_utf8_lossy(&db_content);
            
            let working_lines = split_lines(&working_text);
            let db_lines = split_lines(&db_text);
            
            let edits = myers::diff_lines(&db_lines, &working_lines);
            myers::format_diff(&db_lines, &working_lines, &edits, context_lines)
        }
    };
    
    // Adaugă antetul git-style
    let path_str = file_path.to_string_lossy();
    
    // Calculează hash-ul pentru conținutul de lucru
    let working_hash = database.hash_file_data(&working_content);
    
    // Generează hash-uri scurte pentru a simula formatul git
    let db_hash_short = if oid.len() >= 7 { &oid[0..7] } else { oid };
    let working_hash_short = if working_hash.len() >= 7 { &working_hash[0..7] } else { &working_hash };
    
    // Creează antetul în stil git
    let mut result = String::new();
    result.push_str(&format!("index {}..{} 100644\n", db_hash_short, working_hash_short));
    result.push_str(&format!("--- a/{}\n", path_str));
    result.push_str(&format!("+++ b/{}\n", path_str));
    result.push_str(&diff_content);
    
    Ok(result)
}

/// Compară conținutul din două stringuri
pub fn diff_strings(a: &str, b: &str, context_lines: usize) -> String {
    let a_lines = split_lines(a);
    let b_lines = split_lines(b);
    
    let edits = myers::diff_lines(&a_lines, &b_lines);
    myers::format_diff(&a_lines, &b_lines, &edits, context_lines)
}

/// Colorează ieșirea diff-ului 
pub fn colorize_diff(diff_output: &str) -> String {
    let mut result = String::new();
    
    for line in diff_output.lines() {
        if line.starts_with("@@") && line.contains("@@") {
            // Antet de hunk
            result.push_str(&Color::cyan(line));
            result.push('\n');
        } else if line.starts_with('+') {
            // Linie adăugată
            result.push_str(&Color::green(line));
            result.push('\n');
        } else if line.starts_with('-') {
            // Linie eliminată
            result.push_str(&Color::red(line));
            result.push('\n');
        } else if line.starts_with("Binary") {
            // Mesaje despre fișiere binare
            result.push_str(&Color::yellow(line));
            result.push('\n');
        } else {
            // Linie de context
            result.push_str(line);
            result.push('\n');
        }
    }
    
    result
}