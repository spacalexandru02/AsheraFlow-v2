#[derive(Debug)]
pub enum Command {
    Init { path: String },
    Commit { 
        message: String,
        amend: bool,
        reuse_message: Option<String>,
        edit: bool,
    },
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
    Rm {
        files: Vec<String>,
        cached: bool,
        force: bool,
        recursive: bool,
    },
    Reset {
        files: Vec<String>,
        soft: bool,
        mixed: bool,
        hard: bool,
        force: bool,
        reuse_message: Option<String>,
    },
    CherryPick {
        args: Vec<String>,
        r#continue: bool,
        abort: bool,
        quit: bool,
        mainline: Option<u32>,
    },
    Revert {
        args: Vec<String>,
        r#continue: bool,
        abort: bool,
        quit: bool,
        mainline: Option<u32>,
    },
    // Sprint management commands
    SprintStart {
        name: String,
        duration: u32, // Duration in days
    },
    SprintInfo {},
    // Task management commands
    TaskCreate {
        id: String,
        description: String,
        story_points: Option<u32>,
    },
    TaskComplete {
        id: String,
        story_points: Option<i32>,
        auto_merge: bool,
    },
    TaskStatus {
        id: String,
    },
    Unknown { name: String },
}

#[derive(Debug)]
pub struct CliArgs {
    pub command: Command,
}