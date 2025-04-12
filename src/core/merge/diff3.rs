use std::collections::HashMap;
use std::fmt::Write;
use crate::errors::error::Error;

// Helper to convert a string into a vector of lines with their endings preserved
struct LinesWithEndings<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> LinesWithEndings<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }
}

impl<'a> Iterator for LinesWithEndings<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.input.len() {
            return None;
        }

        let start = self.position;
        
        // Find the next newline character
        while self.position < self.input.len() && !self.input[self.position..].starts_with('\n') {
            self.position += 1;
        }
        
        // Include the newline character if found
        if self.position < self.input.len() {
            self.position += 1;
        }
        
        Some(&self.input[start..self.position])
    }
}

// Simplified representation of edits for our diff3 purposes
#[derive(Debug, PartialEq)]
enum EditType {
    Eql,
    Add,
    Del,
}

#[derive(Debug)]
struct Edit {
    r#type: EditType,
    a_line: Option<LineInfo>,
    b_line: Option<LineInfo>,
}

#[derive(Debug)]
struct LineInfo {
    number: usize,
    content: String,
}

// Function to calculate simple line-by-line diff between two texts
fn diff(a: &str, b: &str) -> Vec<Edit> {
    let a_lines: Vec<_> = a.lines().collect();
    let b_lines: Vec<_> = b.lines().collect();
    
    let mut result = Vec::new();
    let mut i = 0;
    let mut j = 0;
    
    while i < a_lines.len() || j < b_lines.len() {
        if i < a_lines.len() && j < b_lines.len() && a_lines[i] == b_lines[j] {
            // Equal lines
            result.push(Edit {
                r#type: EditType::Eql,
                a_line: Some(LineInfo { number: i, content: a_lines[i].to_string() }),
                b_line: Some(LineInfo { number: j, content: b_lines[j].to_string() }),
            });
            i += 1;
            j += 1;
        } else if j < b_lines.len() && (i >= a_lines.len() || i < a_lines.len() && a_lines[i] != b_lines[j]) {
            // Line added in b
            result.push(Edit {
                r#type: EditType::Add,
                a_line: None,
                b_line: Some(LineInfo { number: j, content: b_lines[j].to_string() }),
            });
            j += 1;
        } else if i < a_lines.len() {
            // Line deleted from a
            result.push(Edit {
                r#type: EditType::Del,
                a_line: Some(LineInfo { number: i, content: a_lines[i].to_string() }),
                b_line: None,
            });
            i += 1;
        }
    }
    
    result
}

/// Performs a three-way merge between original (o), ours (a), and theirs (b) content
pub fn merge(o: &str, a: &str, b: &str) -> Result<MergeResult, Error> {
    let o: Vec<_> = LinesWithEndings::new(o).map(|l| l.to_string()).collect();
    let a: Vec<_> = LinesWithEndings::new(a).map(|l| l.to_string()).collect();
    let b: Vec<_> = LinesWithEndings::new(b).map(|l| l.to_string()).collect();

    let diff3 = Diff3::new(o, a, b);
    diff3.merge()
}

type MatchSet = HashMap<usize, usize>;

#[derive(Debug)]
struct Diff3 {
    o: Vec<String>,
    a: Vec<String>,
    b: Vec<String>,
    chunks: Vec<Chunk>,
    line_o: usize,
    line_a: usize,
    line_b: usize,
    match_a: MatchSet,
    match_b: MatchSet,
}

impl Diff3 {
    pub fn new(o: Vec<String>, a: Vec<String>, b: Vec<String>) -> Self {
        Self {
            o,
            a,
            b,
            chunks: Vec::new(),
            line_o: 0,
            line_a: 0,
            line_b: 0,
            match_a: HashMap::new(),
            match_b: HashMap::new(),
        }
    }

    pub fn merge(mut self) -> Result<MergeResult, Error> {
        self.setup();
        self.generate_chunks();
        Ok(MergeResult::new(self.chunks))
    }

    fn setup(&mut self) {
        self.chunks = Vec::new();
        self.line_o = 0;
        self.line_a = 0;
        self.line_b = 0;

        self.match_a = self.match_set(&self.a);
        self.match_b = self.match_set(&self.b);
    }

    fn match_set(&self, file: &[String]) -> MatchSet {
        let mut matches = HashMap::new();

        // Generate diff between original and this file
        let o_content = self.o.join("");
        let file_content = file.join("");
        
        // Generate diff using our custom diff function
        for edit in diff(&o_content, &file_content) {
            match edit.r#type {
                EditType::Eql => {
                    if let (Some(a_line), Some(b_line)) = (edit.a_line, edit.b_line) {
                        matches.insert(a_line.number, b_line.number);
                    }
                },
                _ => {}
            }
        }

        matches
    }

    #[allow(clippy::unnecessary_unwrap)]
    fn generate_chunks(&mut self) {
        loop {
            let i = self.find_next_mismatch();

            if let Some(i) = i {
                if i == 1 {
                    let (o, a, b) = self.find_next_match();

                    if a.is_some() && b.is_some() {
                        self.emit_chunk(o, a.unwrap(), b.unwrap());
                    } else {
                        self.emit_final_chunk();
                        return;
                    }
                } else {
                    self.emit_chunk(self.line_o + i, self.line_a + i, self.line_b + i);
                }
            } else {
                self.emit_final_chunk();
                return;
            }
        }
    }

    fn find_next_mismatch(&self) -> Option<usize> {
        let mut i = 1;

        while self.in_bounds(i)
            && self.matches(&self.match_a, self.line_a, i)
            && self.matches(&self.match_b, self.line_b, i)
        {
            i += 1;
        }

        if self.in_bounds(i) {
            Some(i)
        } else {
            None
        }
    }

    fn in_bounds(&self, i: usize) -> bool {
        self.line_o + i <= self.o.len()
            || self.line_a + i <= self.a.len()
            || self.line_b + i <= self.b.len()
    }

    fn matches(&self, matches: &MatchSet, offset: usize, i: usize) -> bool {
        matches.get(&(self.line_o + i)) == Some(&(offset + i))
    }

    fn find_next_match(&self) -> (usize, Option<usize>, Option<usize>) {
        let mut o = self.line_o + 1;

        // Find next line in original that's in both other versions
        while o <= self.o.len() && !(self.match_a.contains_key(&o) && self.match_b.contains_key(&o)) {
            o += 1;
        }

        // Return matched line numbers in all three versions
        (
            o,
            self.match_a.get(&o).copied(),
            self.match_b.get(&o).copied(),
        )
    }

    fn emit_chunk(&mut self, o: usize, a: usize, b: usize) {
        // Extract the lines between current position and next match
        let o_lines = self.o[self.line_o..o - 1].to_vec();
        let a_lines = self.a[self.line_a..a - 1].to_vec();
        let b_lines = self.b[self.line_b..b - 1].to_vec();

        self.write_chunk(&o_lines, &a_lines, &b_lines);

        // Update current positions
        self.line_o = o - 1;
        self.line_a = a - 1;
        self.line_b = b - 1;
    }

    fn emit_final_chunk(&mut self) {
        // Extract all remaining lines
        let o_lines = self.o[self.line_o..].to_vec();
        let a_lines = self.a[self.line_a..].to_vec();
        let b_lines = self.b[self.line_b..].to_vec();

        self.write_chunk(&o_lines, &a_lines, &b_lines);
    }

    fn write_chunk(&mut self, o: &[String], a: &[String], b: &[String]) {
        if a == o || a == b {
            // If our version is identical to original or theirs, use theirs
            self.chunks.push(Chunk::Clean { lines: b.to_vec() });
        } else if b == o {
            // If their version is identical to original, use ours
            self.chunks.push(Chunk::Clean { lines: a.to_vec() });
        } else {
            // All versions differ, emit a conflict
            self.chunks.push(Chunk::Conflict {
                o_lines: o.to_vec(),
                a_lines: a.to_vec(),
                b_lines: b.to_vec(),
            });
        }
    }
}

#[derive(Debug, Clone)]
pub enum Chunk {
    Clean {
        lines: Vec<String>,
    },
    Conflict {
        o_lines: Vec<String>,
        a_lines: Vec<String>,
        b_lines: Vec<String>,
    },
}

impl Chunk {
    pub fn to_string(&self, a_name: Option<&str>, b_name: Option<&str>) -> String {
        match self {
            Chunk::Clean { lines } => lines.join(""),
            Chunk::Conflict { o_lines: _, a_lines, b_lines } => {
                fn separator(text: &mut String, r#char: &str, name: Option<&str>) {
                    text.push_str(&r#char.repeat(7));
                    if let Some(name) = name {
                        write!(text, " {}", name).unwrap();
                    }
                    text.push('\n');
                }

                let mut text = String::new();
                separator(&mut text, "<", a_name);
                for line in a_lines {
                    text.push_str(line);
                }
                separator(&mut text, "=", None);
                for line in b_lines {
                    text.push_str(line);
                }
                separator(&mut text, ">", b_name);

                text
            }
        }
    }
}

#[derive(Debug)]
pub struct MergeResult {
    chunks: Vec<Chunk>,
}

impl MergeResult {
    pub fn new(chunks: Vec<Chunk>) -> Self {
        Self { chunks }
    }

    pub fn is_clean(&self) -> bool {
        for chunk in &self.chunks {
            if let Chunk::Conflict { .. } = chunk {
                return false;
            }
        }
        true
    }

    pub fn to_string(&self, a_name: Option<&str>, b_name: Option<&str>) -> String {
        self.chunks
            .iter()
            .map(|chunk| chunk.to_string(a_name, b_name))
            .collect::<Vec<_>>()
            .join("")
    }
}