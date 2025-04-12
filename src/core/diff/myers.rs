// src/core/diff/myers.rs - Implementare corectată și simplificată
use std::cmp;

/// Reprezintă o singură operație de editare într-un diff
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Insert(usize),  // Inserează linia la poziția dată în b
    Delete(usize),  // Șterge linia la poziția dată în a
    Equal(usize, usize), // Liniile sunt egale la pozițiile date în a și b
}

/// Calculează un diff între două secvențe de linii folosind algoritmul Myers optimizat
pub fn diff_lines(a: &[String], b: &[String]) -> Vec<Edit> {
    // Cazul special când ambele fișiere sunt goale
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    
    // Cazul special când un fișier este gol
    if a.is_empty() {
        return b.iter().enumerate()
            .map(|(i, _)| Edit::Insert(i))
            .collect();
    }
    
    if b.is_empty() {
        return a.iter().enumerate()
            .map(|(i, _)| Edit::Delete(i))
            .collect();
    }
    
    // Îmbunătățim diff-ul pentru a asigura identificarea corectă a liniilor comune
    let mut edits = Vec::new();
    let mut i = 0;
    let mut j = 0;
    
    // Abordare liniară pentru găsirea diferențelor între cele două liste de linii
    while i < a.len() || j < b.len() {
        // Verificăm dacă am ajuns la capătul uneia dintre liste
        if i >= a.len() {
            // A s-a terminat, adăugăm toate liniile rămase din B
            edits.push(Edit::Insert(j));
            j += 1;
            continue;
        }
        
        if j >= b.len() {
            // B s-a terminat, adăugăm toate liniile rămase din A ca șterse
            edits.push(Edit::Delete(i));
            i += 1;
            continue;
        }
        
        // Verificăm dacă liniile curente sunt egale
        if a[i] == b[j] {
            // Liniile sunt egale, le marcăm ca atare
            edits.push(Edit::Equal(i, j));
            i += 1;
            j += 1;
        } else {
            // Liniile sunt diferite, verificăm dacă putem găsi o potrivire în următoarele linii
            // Încercăm să găsim linia curentă din A în viitoarele linii din B
            let mut found_in_b = false;
            for look_ahead in 1..=3 { // Limităm căutarea înainte pentru eficiență
                if j + look_ahead < b.len() && a[i] == b[j + look_ahead] {
                    // Am găsit linia din A mai târziu în B - înseamnă că avem inserții în B
                    for k in 0..look_ahead {
                        edits.push(Edit::Insert(j + k));
                    }
                    j += look_ahead;
                    found_in_b = true;
                    break;
                }
            }
            
            if !found_in_b {
                // Încercăm să găsim linia curentă din B în viitoarele linii din A
                let mut found_in_a = false;
                for look_ahead in 1..=3 { // Limităm căutarea înainte pentru eficiență
                    if i + look_ahead < a.len() && b[j] == a[i + look_ahead] {
                        // Am găsit linia din B mai târziu în A - înseamnă că avem ștergeri în A
                        for k in 0..look_ahead {
                            edits.push(Edit::Delete(i + k));
                        }
                        i += look_ahead;
                        found_in_a = true;
                        break;
                    }
                }
                
                if !found_in_a {
                    // Nu am găsit potriviri în look-ahead - trebuie să considerăm o linie ștearsă din A și una adăugată în B
                    edits.push(Edit::Delete(i));
                    i += 1;
                    edits.push(Edit::Insert(j));
                    j += 1;
                }
            }
        }
    }
    
    edits
}

/// Determină dacă un fișier este binar (conține caractere nul sau un procent ridicat de caractere non-text)
pub fn is_binary_content(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }
    
    // Dacă conține octeți nul, este probabil binar
    if content.contains(&0) {
        return true;
    }
    
    // Verifică un eșantion pentru a determina dacă este probabil binar
    // Analizăm primele ~8KB pentru a decide
    let sample_size = std::cmp::min(8192, content.len());
    let sample = &content[0..sample_size];
    
    // Numără octeții care nu sunt caractere ASCII imprimabile sau whitespace
    let non_text = sample.iter().filter(|&&b| !b.is_ascii_graphic() && !b.is_ascii_whitespace()).count();
    
    // Dacă mai mult de 30% din conținut este non-text, considerăm fișierul binar
    (non_text as f64 / sample_size as f64) > 0.3
}

/// Format a diff for display, git-style with improved hunk calculation
pub fn format_diff(a: &[String], b: &[String], edits: &[Edit], context_lines: usize) -> String {
    let mut result = String::new();
    
    // Verifică dacă avem operații de editare
    if edits.is_empty() {
        return result;
    }
    
    // Cazul special pentru fișiere goale
    if a.is_empty() && !b.is_empty() {
        // Adăugare de conținut la un fișier gol
        result.push_str("@@ -0,0 +1,");
        result.push_str(&b.len().to_string());
        result.push_str(" @@\n");
        
        for line in b {
            result.push_str(&format!("+{}\n", line));
        }
        
        return result;
    } else if !a.is_empty() && b.is_empty() {
        // Ștergerea întregului conținut
        result.push_str("@@ -1,");
        result.push_str(&a.len().to_string());
        result.push_str(" +0,0 @@\n");
        
        for line in a {
            result.push_str(&format!("-{}\n", line));
        }
        
        return result;
    } else if a.is_empty() && b.is_empty() {
        // Ambele fișiere sunt goale
        return result;
    }
    
    // Reconstruim un model de diferențe linie cu linie pentru a avea o vizualizare mai clară
    let mut line_model = Vec::new();
    
    // Trackers for line positions
    let mut a_pos = 0;
    let mut b_pos = 0;
    
    // Construim modelul de linii, ținând cont de toate operațiile de edit
    for edit in edits {
        match edit {
            Edit::Equal(a_idx, b_idx) => {
                // Adăugăm toate liniile egale omise între ultima poziție și cea curentă
                while a_pos < *a_idx && b_pos < *b_idx {
                    // Decidem ce să facem cu liniile "sărind peste" - cel mai probabil adăugări/ștergeri
                    if a_pos == *a_idx - (b_idx - b_pos) {
                        // Linii adăugate în B
                        for i in b_pos..*b_idx {
                            line_model.push(('I', None, Some(i))); // Insert
                        }
                        b_pos = *b_idx;
                    } else if b_pos == *b_idx - (a_idx - a_pos) {
                        // Linii șterse din A
                        for i in a_pos..*a_idx {
                            line_model.push(('D', Some(i), None)); // Delete
                        }
                        a_pos = *a_idx;
                    } else {
                        // Ambele avansează cât mai mult posibil
                        a_pos += 1;
                        b_pos += 1;
                    }
                }
                
                // Adăugăm linia egală curentă
                line_model.push(('E', Some(*a_idx), Some(*b_idx))); // Equal
                a_pos = a_idx + 1;
                b_pos = b_idx + 1;
            },
            Edit::Delete(a_idx) => {
                // Adăugăm liniile egale omise
                while a_pos < *a_idx {
                    if b_pos < b.len() && a[a_pos] == b[b_pos] {
                        line_model.push(('E', Some(a_pos), Some(b_pos)));
                        a_pos += 1;
                        b_pos += 1;
                    } else {
                        line_model.push(('D', Some(a_pos), None));
                        a_pos += 1;
                    }
                }
                
                // Adăugăm linia ștearsă curentă
                line_model.push(('D', Some(*a_idx), None));
                a_pos = a_idx + 1;
            },
            Edit::Insert(b_idx) => {
                // Adăugăm liniile egale omise
                while b_pos < *b_idx {
                    if a_pos < a.len() && a[a_pos] == b[b_pos] {
                        line_model.push(('E', Some(a_pos), Some(b_pos)));
                        a_pos += 1;
                        b_pos += 1;
                    } else {
                        line_model.push(('I', None, Some(b_pos)));
                        b_pos += 1;
                    }
                }
                
                // Adăugăm linia inserată curentă
                line_model.push(('I', None, Some(*b_idx)));
                b_pos = b_idx + 1;
            }
        }
    }
    
    // Adăugăm orice linii rămase
    while a_pos < a.len() || b_pos < b.len() {
        if a_pos < a.len() && b_pos < b.len() && a[a_pos] == b[b_pos] {
            line_model.push(('E', Some(a_pos), Some(b_pos)));
            a_pos += 1;
            b_pos += 1;
        } else if a_pos < a.len() {
            line_model.push(('D', Some(a_pos), None));
            a_pos += 1;
        } else if b_pos < b.len() {
            line_model.push(('I', None, Some(b_pos)));
            b_pos += 1;
        }
    }
    
    // Identificăm hunk-uri - grupuri de linii cu cel puțin o modificare
    let mut hunks = Vec::new();
    let mut current_hunk = Vec::new();
    let mut prev_change_idx = None;
    
    for (idx, (op, _, _)) in line_model.iter().enumerate() {
        let is_change = *op == 'I' || *op == 'D';
        
        if is_change {
            // Marcăm indexul pentru această schimbare
            prev_change_idx = Some(idx);
            
            // Adăugăm linii de context înainte dacă nu le-am adăugat deja
            let start_context = if idx > context_lines {
                idx - context_lines
            } else {
                0
            };
            
            // Adăugăm context anterior
            for context_idx in start_context..idx {
                if !current_hunk.contains(&context_idx) {
                    current_hunk.push(context_idx);
                }
            }
            
            // Adăugăm linia curentă
            current_hunk.push(idx);
        } else if let Some(prev_idx) = prev_change_idx {
            // Aceasta este o linie de context după o schimbare
            if idx - prev_idx <= context_lines {
                // Linie de context în limita distanței
                current_hunk.push(idx);
            } else {
                // Am depășit distanța de context - finalizăm hunk-ul curent
                if !current_hunk.is_empty() {
                    // Sortăm și eliminăm duplicatele
                    current_hunk.sort();
                    current_hunk.dedup();
                    hunks.push(current_hunk);
                    current_hunk = Vec::new();
                }
                
                // Resetăm indexul ultimei schimbări
                prev_change_idx = None;
            }
        }
    }
    
    // Adăugăm ultimul hunk rămas
    if !current_hunk.is_empty() {
        current_hunk.sort();
        current_hunk.dedup();
        hunks.push(current_hunk);
    }
    
    // Dacă nu avem hunk-uri dar avem linii în fișiere, afișăm totul ca fiind neschimbat
    if hunks.is_empty() && !a.is_empty() {
        result.push_str("@@ -1,");
        result.push_str(&a.len().to_string());
        result.push_str(" +1,");
        result.push_str(&b.len().to_string());
        result.push_str(" @@\n");
        
        for i in 0..std::cmp::min(a.len(), b.len()) {
            result.push_str(&format!(" {}\n", a[i]));
        }
        
        return result;
    }
    
    // Formatăm fiecare hunk
    for hunk_indices in hunks {
        if hunk_indices.is_empty() {
            continue;
        }
        
        // Calculăm intervalele de linii în ambele fișiere
        let mut a_min = usize::MAX;
        let mut a_max = 0;
        let mut b_min = usize::MAX;
        let mut b_max = 0;
        
        for &idx in &hunk_indices {
            if idx >= line_model.len() {
                continue;
            }
            
            let (_, a_idx, b_idx) = line_model[idx];
            
            if let Some(a_i) = a_idx {
                a_min = a_min.min(a_i);
                a_max = a_max.max(a_i + 1);
            }
            
            if let Some(b_i) = b_idx {
                b_min = b_min.min(b_i);
                b_max = b_max.max(b_i + 1);
            }
        }
        
        // Corectăm valorile min/max dacă nu am găsit referințe
        if a_min == usize::MAX {
            a_min = 0;
        }
        if b_min == usize::MAX {
            b_min = 0;
        }
        
        let a_count = a_max - a_min;
        let b_count = b_max - b_min;
        
        // Adăugăm header-ul hunk-ului
        result.push_str(&format!("@@ -{},{} +{},{} @@\n", 
                          a_min + 1, a_count, b_min + 1, b_count));
        
        // Formatăm liniile în hunk
        for &idx in &hunk_indices {
            if idx >= line_model.len() {
                continue;
            }
            
            let (op, a_idx, b_idx) = line_model[idx];
            
            match op {
                'E' => {
                    // Linie egală (prezentă în ambele fișiere)
                    if let Some(a_i) = a_idx {
                        if a_i < a.len() {
                            result.push_str(&format!(" {}\n", a[a_i]));
                        }
                    }
                },
                'D' => {
                    // Linie ștearsă (prezentă doar în A)
                    if let Some(a_i) = a_idx {
                        if a_i < a.len() {
                            result.push_str(&format!("-{}\n", a[a_i]));
                        }
                    }
                },
                'I' => {
                    // Linie inserată (prezentă doar în B)
                    if let Some(b_i) = b_idx {
                        if b_i < b.len() {
                            result.push_str(&format!("+{}\n", b[b_i]));
                        }
                    }
                },
                _ => {}
            }
        }
    }
    
    result
}