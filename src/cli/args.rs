#[derive(Debug)]
pub enum Command {
    Init { path: String },
    Commit { message: String },
    Add { paths: Vec<String> },
    Status { porcelain: bool, color: String }, 
    Diff { paths: Vec<String>, cached: bool },
    Branch { 
        name: String, 
        start_point: Option<String>,
        verbose: bool,
        delete: bool,
        force: bool
    },
    Checkout { target: String },
    Log {
        revisions: Vec<String>,
        abbrev: bool,
        format: String,
        patch: bool,
        decorate: String,
    },
    Merge {
        branch: String,
        message: Option<String>,
        abort: bool,
        continue_merge: bool,
        tool: Option<String>, 
    },
    Unknown { name: String },
}

#[derive(Debug)]
pub struct CliArgs {
    pub command: Command,
}