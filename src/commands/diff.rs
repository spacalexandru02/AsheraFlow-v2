// src/commands/diff.rs - updated to use pager
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::time::Instant;
use crate::core::color::Color;
use crate::core::database::database::Database;
use crate::core::database::tree::{Tree, TreeEntry, TREE_MODE};
use crate::core::index::index::Index;
use crate::core::database::commit::Commit;
use crate::core::refs::Refs;
use crate::core::workspace::Workspace;
use crate::core::diff::diff;
use crate::core::diff::myers::{diff_lines, format_diff, is_binary_content};
use crate::errors::error::Error;
use crate::core::pager::Pager;

pub struct DiffCommand;

impl DiffCommand {
    /// Execute diff command between index/HEAD and working tree
    pub fn execute(paths: &[String], cached: bool) -> Result<(), Error> {
        let start_time = Instant::now();
        
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        
        // Verifică dacă directorul .ash există
        if !git_path.exists() {
            return Err(Error::Generic("fatal: not an ash repository (or any of the parent directories): .ash directory not found".into()));
        }
        
        let workspace = Workspace::new(root_path);
        let mut database = Database::new(git_path.join("objects"));
        let mut index = Index::new(git_path.join("index"));
        
        // Load the index first
        index.load()?;
        
        let refs = Refs::new(&git_path);
        
        // Initialize the pager
        let mut pager = Pager::new();
        
        // Start the pager - this creates the pager process
        pager.start()?;
        
        // Execute diff commands
        let result = if paths.is_empty() {
            // Treat the entire repository
            Self::diff_all(&workspace, &mut database, &index, &refs, cached, &mut pager)
        } else {
            // Process specific paths
            let mut overall_result = Ok(());
            
            for path_str in paths {
                // Stop processing if user exited pager
                if !pager.is_enabled() {
                    break;
                }
                
                let path = PathBuf::from(path_str);
                if let Err(e) = Self::diff_path(&workspace, &mut database, &index, &refs, &path, cached, &mut pager) {
                    overall_result = Err(e);
                    break;
                }
            }
            
            overall_result
        };
        
        // Only show completion message if pager is still active (user hasn't exited)
        if pager.is_enabled() {
            let elapsed = start_time.elapsed();
            let _ = pager.write(&format!("\n{}\n", Color::cyan(&format!("Diff completed in {:.2}s", elapsed.as_secs_f32()))));
        }
        
        // Close pager properly - this will wait for it to exit if it's still running
        let close_result = pager.close();
        
        // Return the first error we encountered (either from diff or closing pager)
        match (result, close_result) {
            (Err(e), _) => Err(e),
            (_, Err(e)) => Err(e),
            _ => Ok(()),
        }
    }

    /// Diff all changed files in the repository
    fn diff_all(
        workspace: &Workspace,
        database: &mut Database,
        index: &Index,
        refs: &Refs,
        cached: bool,
        pager: &mut Pager
    ) -> Result<(), Error> {
        // Dacă flag-ul cached este setat, compară indexul cu HEAD
        if cached {
            return Self::diff_index_vs_head(workspace, database, index, refs, pager);
        }
        
        // În caz contrar, compară arborele de lucru cu indexul
        let mut has_changes = false;
        
        // Obține toate fișierele din index
        for entry in index.each_entry() {
            let path = Path::new(entry.get_path());
            
            // Sări dacă fișierul nu există în workspace
            if !workspace.path_exists(path)? {
                has_changes = true;
                let path_str = path.display().to_string();
                pager.write(&format!("diff --ash a/{} b/{}\n", Color::cyan(&path_str), Color::cyan(&path_str)))?;
                pager.write(&format!("{} {}\n", Color::red("deleted file mode"), Color::red(&entry.mode_octal())))?;
                pager.write(&format!("--- a/{}\n", Color::red(&path_str)))?;
                pager.write(&format!("+++ {}\n", Color::red("/dev/null")))?;
                
                // Obține conținutul blob-ului din baza de date
                let blob_obj = database.load(entry.get_oid())?;
                let content = blob_obj.to_bytes();
                
                // Verifică dacă conținutul este binar
                if is_binary_content(&content) {
                    pager.write(&format!("Binary file a/{} has been deleted\n", path_str))?;
                    continue;
                }
                
                let lines = diff::split_lines(&String::from_utf8_lossy(&content));
                
                // Arată diff-ul de ștergere
                for line in &lines {
                    pager.write(&format!("{}\n", Color::red(&format!("-{}", line))))?;
                }
                
                continue;
            }
            
            // Citește conținutul fișierului
            let file_content = workspace.read_file(path)?;
            
            // Calculează hash-ul pentru conținutul fișierului
            let file_hash = database.hash_file_data(&file_content);
            
            // Dacă hash-ul se potrivește, nu există nicio modificare
            if file_hash == entry.get_oid() {
                continue;
            }
            
            has_changes = true;
            
            // Tipărește antetul diff-ului
            let path_str = path.display().to_string();
            pager.write(&format!("diff --ash a/{} b/{}\n", Color::cyan(&path_str), Color::cyan(&path_str)))?;
            
            // Verifică dacă fișierul este binar
            if is_binary_content(&file_content) {
                pager.write(&format!("Binary files a/{} and b/{} differ\n", path_str, path_str))?;
                continue;
            }
            
            // Obține diff-ul între index și copia de lucru
            let raw_diff_output = diff::diff_with_database(workspace, database, path, entry.get_oid(), 3)?;
            
            // Adaugă culori la ieșirea diff-ului
            let colored_diff = Self::colorize_diff_output(&raw_diff_output);
            pager.write(&colored_diff)?;
        }
        
        if !has_changes {
            pager.write(&format!("{}\n", Color::green("No changes")))?;
        }
        
        Ok(())
    }

    /// Metodă helper pentru colorarea ieșirii diff-ului
    fn colorize_diff_output(diff: &str) -> String {
        let mut result = String::new();
        
        for line in diff.lines() {
            if line.starts_with("Binary files") {
                // Mesaje despre fișiere binare
                result.push_str(&Color::yellow(line));
                result.push('\n');
            } else if line.starts_with("@@") && line.contains("@@") {
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
            } else {
                // Linie de context
                result.push_str(line);
                result.push('\n');
            }
        }
        
        result
    }

    /// Colectează toate fișierele dintr-un commit
    fn collect_files_from_commit(
        database: &mut Database,
        commit: &Commit,
        files: &mut HashMap<String, String>
    ) -> Result<(), Error> {
        // Obține OID-ul arborelui din commit
        let tree_oid = commit.get_tree();
        
        // Colectează fișierele din arbore
        Self::collect_files_from_tree(database, tree_oid, PathBuf::new(), files)?;
        
        Ok(())
    }

    // Implementare îmbunătățită pentru a trata recursiv traversarea arborilor
    fn collect_files_from_tree(
        database: &mut Database,
        tree_oid: &str,
        prefix: PathBuf,
        files: &mut HashMap<String, String>
    ) -> Result<(), Error> {
        // Încarcă obiectul
        let obj = match database.load(tree_oid) {
            Ok(obj) => obj,
            Err(e) => {
                println!("Warning: Could not load object {}: {}", tree_oid, e);
                return Ok(());
            }
        };
        
        // Verifică dacă obiectul este un arbore
        if let Some(tree) = obj.as_any().downcast_ref::<Tree>() {
            // Procesează fiecare intrare din arbore
            for (name, entry) in tree.get_entries() {
                let entry_path = if prefix.as_os_str().is_empty() {
                    PathBuf::from(name)
                } else {
                    prefix.join(name)
                };
                
                let entry_path_str = entry_path.to_string_lossy().to_string();
                
                match entry {
                    TreeEntry::Blob(oid, mode) => {
                        // Dacă aceasta este o intrare de director deghizată ca blob
                        if *mode == TREE_MODE || mode.is_directory() {
                            // Procesează recursiv acest director
                            if let Err(e) = Self::collect_files_from_tree(database, oid, entry_path, files) {
                                println!("Warning: Error traversing directory '{}': {}", entry_path_str, e);
                            }
                        } else {
                            // Fișier normal
                            files.insert(entry_path_str, oid.clone());
                        }
                    },
                    TreeEntry::Tree(subtree) => {
                        if let Some(subtree_oid) = subtree.get_oid() {
                            // Procesează recursiv acest director
                            if let Err(e) = Self::collect_files_from_tree(database, subtree_oid, entry_path, files) {
                                println!("Warning: Error traversing subtree '{}': {}", entry_path_str, e);
                            }
                        }
                    }
                }
            }
            
            return Ok(());
        }
        
        // Dacă obiectul este un blob, încearcă să-l parsezi ca arbore
        if obj.get_type() == "blob" {
            // Încearcă să parsezi blob-ul ca arbore (aceasta tratează directoare stocate ca blob-uri)
            let blob_data = obj.to_bytes();
            if let Ok(parsed_tree) = Tree::parse(&blob_data) {
                // Procesează fiecare intrare din arborele parsat
                for (name, entry) in parsed_tree.get_entries() {
                    let entry_path = if prefix.as_os_str().is_empty() {
                        PathBuf::from(name)
                    } else {
                        prefix.join(name)
                    };
                    
                    let entry_path_str = entry_path.to_string_lossy().to_string();
                    
                    match entry {
                        TreeEntry::Blob(oid, mode) => {
                            if *mode == TREE_MODE || mode.is_directory() {
                                // Procesează recursiv acest director
                                if let Err(e) = Self::collect_files_from_tree(database, oid, entry_path, files) {
                                    println!("Warning: Error traversing directory '{}': {}", entry_path_str, e);
                                }
                            } else {
                                // Fișier normal
                                files.insert(entry_path_str, oid.clone());
                            }
                        },
                        TreeEntry::Tree(subtree) => {
                            if let Some(subtree_oid) = subtree.get_oid() {
                                // Procesează recursiv acest director
                                if let Err(e) = Self::collect_files_from_tree(database, subtree_oid, entry_path, files) {
                                    println!("Warning: Error traversing subtree '{}': {}", entry_path_str, e);
                                }
                            }
                        }
                    }
                }
                
                return Ok(());
            } else {
                // Dacă suntem la o cale non-root, acesta ar putea fi un fișier
                if !prefix.as_os_str().is_empty() {
                    let path_str = prefix.to_string_lossy().to_string();
                    files.insert(path_str, tree_oid.to_string());
                    return Ok(());
                }
            }
        }
        
        // Caz special pentru intrări de top-level care ar putea necesita traversare mai profundă
        if prefix.as_os_str().is_empty() {
            // Verifică toate intrările găsite în root
            for (path, oid) in files.clone() {  // Clonăm pentru a evita probleme de împrumut
                // Doar căutăm intrări de director de top-level (fără separatori de cale)
                if !path.contains('/') {
                    // Încearcă să încarci și să traversezi ca director
                    let dir_path = PathBuf::from(&path);
                    if let Err(e) = Self::collect_files_from_tree(database, &oid, dir_path, files) {
                        println!("Warning: Error traversing entry '{}': {}", path, e);
                        // Continuă cu alte intrări chiar dacă aceasta eșuează
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Diff a specific path
    fn diff_path(
        workspace: &Workspace,
        database: &mut Database,
        index: &Index,
        refs: &Refs,
        path: &Path,
        cached: bool,
        pager: &mut Pager
    ) -> Result<(), Error> {
        let path_str = path.to_string_lossy().to_string();
        
        // Dacă calea este în index
        if let Some(entry) = index.get_entry(&path_str) {
            if cached {
                // Compară indexul cu HEAD
                let head_oid = match refs.read_head()? {
                    Some(oid) => oid,
                    None => {
                        // Fără HEAD, arată ca fișier nou
                        let index_obj = database.load(entry.get_oid())?;
                        let content = index_obj.to_bytes();
                        
                        // Verifică dacă fișierul este binar
                        if is_binary_content(&content) {
                            pager.write(&format!("Binary file b/{} created\n", path_str))?;
                            return Ok(());
                        }
                        
                        // Generează un hash fictiv pentru formatul git
                        let index_hash = entry.get_oid();
                        let index_hash_short = if index_hash.len() >= 7 { &index_hash[0..7] } else { index_hash };
                        
                        pager.write(&format!("index 0000000..{} 100644\n", index_hash_short))?;
                        pager.write(&format!("--- /dev/null\n"))?;
                        pager.write(&format!("+++ b/{}\n", path_str))?;
                        pager.write(&format!("@@ -0,0 +1,{} @@\n", content.len()))?;
                        
                        let lines = diff::split_lines(&String::from_utf8_lossy(&content));
                        
                        for line in &lines {
                            pager.write(&format!("{}\n", Color::green(&format!("+{}", line))))?;
                        }
                        
                        return Ok(());
                    }
                };
                
                // Obține fișierul din commit-ul HEAD
                let commit_obj = database.load(&head_oid)?;
                let commit = match commit_obj.as_any().downcast_ref::<Commit>() {
                    Some(c) => c,
                    None => return Err(Error::Generic("HEAD is not a commit".into())),
                };
                
                let mut head_files: HashMap<String, String> = HashMap::new();
                DiffCommand::collect_files_from_commit(database, commit, &mut head_files)?;
                
                if let Some(head_oid) = head_files.get(&path_str) {
                    // Fișierul există atât în HEAD, cât și în index
                    if head_oid == entry.get_oid() {
                        pager.write(&format!("{}\n", Color::green(&format!("No changes staged for {}", path_str))))?;
                        return Ok(());
                    }
                    
                    // Compară versiunile din HEAD și index
                    // Încarcă ambele versiuni
                    let head_obj = database.load(head_oid)?;
                    let index_obj = database.load(entry.get_oid())?;
                    
                    let head_content = head_obj.to_bytes();
                    let index_content = index_obj.to_bytes();
                    
                    // Verifică dacă vreunul dintre fișiere este binar
                    if is_binary_content(&head_content) || is_binary_content(&index_content) {
                        pager.write(&format!("Binary files a/{} and b/{} differ\n", path_str, path_str))?;
                        return Ok(());
                    }
                    
                    // Generează hash-uri scurte pentru formatul git
                    let head_hash_short = if head_oid.len() >= 7 { &head_oid[0..7] } else { head_oid };
                    let index_hash_short = if entry.get_oid().len() >= 7 { &entry.get_oid()[0..7] } else { entry.get_oid() };
                    
                    pager.write(&format!("index {}..{} {}\n", head_hash_short, index_hash_short, entry.mode_octal()))?;
                    pager.write(&format!("--- a/{}\n", path_str))?;
                    pager.write(&format!("+++ b/{}\n", path_str))?;
                    
                    let head_lines = diff::split_lines(&String::from_utf8_lossy(&head_content));
                    let index_lines = diff::split_lines(&String::from_utf8_lossy(&index_content));
                    
                    // Calculează diff-ul
                    let edits = diff_lines(&head_lines, &index_lines);
                    let diff_text = format_diff(&head_lines, &index_lines, &edits, 3);
                    
                    // Afișează diff-ul colorat
                    pager.write(&DiffCommand::colorize_diff_output(&diff_text))?;
                } else {
                    // Fișierul este în index, dar nu în HEAD (fișier nou)
                    let index_obj = database.load(entry.get_oid())?;
                    let content = index_obj.to_bytes();
                    
                    // Verifică dacă fișierul este binar
                    if is_binary_content(&content) {
                        pager.write(&format!("Binary file b/{} created\n", path_str))?;
                        return Ok(());
                    }
                    
                    // Generează un hash fictiv pentru formatul git
                    let index_hash = entry.get_oid();
                    let index_hash_short = if index_hash.len() >= 7 { &index_hash[0..7] } else { index_hash };
                    
                    pager.write(&format!("index 0000000..{} {}\n", index_hash_short, entry.mode_octal()))?;
                    pager.write(&format!("--- /dev/null\n"))?;
                    pager.write(&format!("+++ b/{}\n", path_str))?;
                    pager.write(&format!("@@ -0,0 +1,{} @@\n", content.len()))?;
                    
                    let lines = diff::split_lines(&String::from_utf8_lossy(&content));
                    
                    for line in &lines {
                        pager.write(&format!("{}\n", Color::green(&format!("+{}", line))))?;
                    }
                }
            } else {
                // Compară indexul cu arborele de lucru
                if !workspace.path_exists(path)? {
                    let index_obj = database.load(entry.get_oid())?;
                    let content = index_obj.to_bytes();
                    
                    // Verifică dacă fișierul este binar
                    if is_binary_content(&content) {
                        pager.write(&format!("Binary file a/{} has been deleted\n", path_str))?;
                        return Ok(());
                    }
                    
                    // Generează un hash fictiv pentru formatul git
                    let index_hash = entry.get_oid();
                    let index_hash_short = if index_hash.len() >= 7 { &index_hash[0..7] } else { index_hash };
                    
                    pager.write(&format!("index {}..0000000 {}\n", index_hash_short, entry.mode_octal()))?;
                    pager.write(&format!("--- a/{}\n", path_str))?;
                    pager.write(&format!("+++ /dev/null\n"))?;
                    pager.write(&format!("@@ -1,{} +0,0 @@\n", content.len()))?;
                    
                    let lines = diff::split_lines(&String::from_utf8_lossy(&content));
                    
                    for line in &lines {
                        pager.write(&format!("{}\n", Color::red(&format!("-{}", line))))?;
                    }
                    
                    return Ok(());
                }
                
                // Citește copia de lucru
                let file_content = workspace.read_file(path)?;
                
                // Calculează hash-ul pentru conținutul fișierului
                let file_hash = database.hash_file_data(&file_content);
                
                // Dacă hash-ul se potrivește, nu există nicio modificare
                if file_hash == entry.get_oid() {
                    pager.write(&format!("{}\n", Color::green(&format!("No changes in {}", path_str))))?;
                    return Ok(());
                }
                
                // Verifică dacă fișierul este binar
                if is_binary_content(&file_content) {
                    pager.write(&format!("index {}..{} {}\n", 
                            &entry.get_oid()[0..std::cmp::min(7, entry.get_oid().len())], 
                            &file_hash[0..std::cmp::min(7, file_hash.len())], 
                            entry.mode_octal()))?;
                    pager.write(&format!("Binary files a/{} and b/{} differ\n", path_str, path_str))?;
                    return Ok(());
                }
                
                // Arată diff-ul între index și copia de lucru
                // Generează hash-uri scurte pentru formatul git
                let index_hash_short = if entry.get_oid().len() >= 7 { &entry.get_oid()[0..7] } else { entry.get_oid() };
                let file_hash_short = if file_hash.len() >= 7 { &file_hash[0..7] } else { &file_hash };
                
                pager.write(&format!("index {}..{} {}\n", index_hash_short, file_hash_short, entry.mode_octal()))?;
                pager.write(&format!("--- a/{}\n", path_str))?;
                pager.write(&format!("+++ b/{}\n", path_str))?;
                
                // Folosește diff_with_database din modulul diff pentru a obține conținutul diff-ului
                let raw_diff_output = diff::diff_with_database(workspace, database, path, entry.get_oid(), 3)?;
                
                // Extrage doar partea cu diferențele (fără antetele adăugate de diff_with_database)
                let lines: Vec<&str> = raw_diff_output.lines().collect();
                let diff_content = if lines.len() > 3 {
                    // Sari peste primele 3 linii (antetele) care sunt deja afișate
                    lines[3..].join("\n")
                } else {
                    raw_diff_output
                };
                
                // Colorează și afișează diff-ul
                pager.write(&DiffCommand::colorize_diff_output(&diff_content))?;
            }
        } else {
            // Calea nu este în index
            if workspace.path_exists(path)? {
                pager.write(&format!("{}\n", Color::red(&format!("error: path '{}' is untracked", path_str))))?;
            } else {
                pager.write(&format!("{}\n", Color::red(&format!("error: path '{}' does not exist", path_str))))?;
            }
        }
        
        Ok(())
    }

    fn diff_index_vs_head(
        workspace: &Workspace,
        database: &mut Database,
        index: &Index,
        refs: &Refs,
        pager: &mut Pager
    ) -> Result<(), Error> {
        // Obține commit-ul HEAD
        let head_oid = match refs.read_head()? {
            Some(oid) => oid,
            None => {
                pager.write(&format!("{}\n", Color::yellow("No HEAD commit found. Index contains initial version.")))?;
                return Ok(());
            }
        };
        
        // Încarcă commit-ul HEAD
        let commit_obj = database.load(&head_oid)?;
        let commit = match commit_obj.as_any().downcast_ref::<Commit>() {
            Some(c) => c,
            None => return Err(Error::Generic("HEAD is not a commit".into())),
        };
        
        // Obține fișierele din HEAD
        let mut head_files: HashMap<String, String> = HashMap::new();
        DiffCommand::collect_files_from_commit(database, commit, &mut head_files)?;
        
        let mut has_changes = false;
        
        // Compară fișierele din index cu HEAD
        for entry in index.each_entry() {
            let path = entry.get_path();
            
            if let Some(head_oid) = head_files.get(path) {
                // Fișierul există atât în index, cât și în HEAD
                if head_oid == entry.get_oid() {
                    // Nicio modificare
                    continue;
                }
                
                // Fișierul a fost modificat
                has_changes = true;
                
                // Generează hash-uri scurte pentru antetul git
                let head_hash_short = if head_oid.len() >= 7 { &head_oid[0..7] } else { head_oid };
                let index_hash_short = if entry.get_oid().len() >= 7 { &entry.get_oid()[0..7] } else { entry.get_oid() };
                
                pager.write(&format!("index {}..{} {}\n", head_hash_short, index_hash_short, entry.mode_octal()))?;
                pager.write(&format!("--- a/{}\n", path))?;
                pager.write(&format!("+++ b/{}\n", path))?;
                
                // Încarcă ambele versiuni
                let head_obj = database.load(head_oid)?;
                let index_obj = database.load(entry.get_oid())?;
                
                let head_content = head_obj.to_bytes();
                let index_content = index_obj.to_bytes();
                
                // Verifică dacă fișierul este binar
                if is_binary_content(&head_content) || is_binary_content(&index_content) {
                    pager.write(&format!("Binary files a/{} and b/{} differ\n", path, path))?;
                    continue;
                }
                
                let head_lines = diff::split_lines(&String::from_utf8_lossy(&head_content));
                let index_lines = diff::split_lines(&String::from_utf8_lossy(&index_content));
                
                // Calculează diff-ul
                let edits = diff_lines(&head_lines, &index_lines);
                let raw_diff = format_diff(&head_lines, &index_lines, &edits, 3);
                
                // Colorează și afișează diff-ul
                let colored_diff = DiffCommand::colorize_diff_output(&raw_diff);
                pager.write(&colored_diff)?;
            } else {
                // Fișierul există în index, dar nu în HEAD (fișier nou)
                has_changes = true;
                
                // Generează hash-ul pentru antetul git
                let index_hash_short = if entry.get_oid().len() >= 7 { &entry.get_oid()[0..7] } else { entry.get_oid() };
                
                pager.write(&format!("index 0000000..{} {}\n", index_hash_short, entry.mode_octal()))?;
                pager.write(&format!("--- /dev/null\n"))?;
                pager.write(&format!("+++ b/{}\n", path))?;
                
                // Încarcă versiunea din index
                let index_obj = database.load(entry.get_oid())?;
                let content = index_obj.to_bytes();
                
                // Verifică dacă fișierul este binar
                if is_binary_content(&content) {
                    pager.write(&format!("Binary file b/{} created\n", path))?;
                    continue;
                }
                
                let lines = diff::split_lines(&String::from_utf8_lossy(&content));
                
                // Afișează antetul hunk-ului
                pager.write(&format!("@@ -0,0 +1,{} @@\n", lines.len()))?;
                
                // Arată diff-ul de adăugare
                for line in &lines {
                    pager.write(&format!("{}\n", Color::green(&format!("+{}", line))))?;
                }
            }
        }
        
        // Verifică fișierele din HEAD care au fost eliminate din index
        for (path, head_oid) in &head_files {
            if !index.tracked(path) {
                // Fișierul a fost în HEAD, dar a fost eliminat din index
                has_changes = true;
                
                // Generează hash-ul pentru antetul git
                let head_hash_short = if head_oid.len() >= 7 { &head_oid[0..7] } else { head_oid };
                
                pager.write(&format!("index {}..0000000\n", head_hash_short))?;
                pager.write(&format!("--- a/{}\n", path))?;
                pager.write(&format!("+++ /dev/null\n"))?;
                
                // Încarcă versiunea din HEAD
                let head_obj = database.load(head_oid)?;
                let content = head_obj.to_bytes();
                
                // Verifică dacă fișierul este binar
                if is_binary_content(&content) {
                    pager.write(&format!("Binary file a/{} deleted\n", path))?;
                    continue;
                }
                
                let lines = diff::split_lines(&String::from_utf8_lossy(&content));
                
                // Afișează antetul hunk-ului
                pager.write(&format!("@@ -1,{} +0,0 @@\n", lines.len()))?;
                
                // Arată diff-ul de ștergere
                for line in &lines {
                    pager.write(&format!("{}\n", Color::red(&format!("-{}", line))))?;
                }
            }
        }
        
        if !has_changes {
            pager.write(&format!("{}\n", Color::green("No changes staged for commit")))?;
        }
        
        Ok(())
    }  
}