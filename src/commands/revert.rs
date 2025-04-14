use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::database::commit::Commit;
use crate::core::database::database::Database;
use crate::core::editor::Editor;
use crate::core::refs::{Refs, HEAD};
use crate::errors::error::Error;
use crate::core::index::index::Index;
use crate::core::merge::inputs;
use crate::core::merge::resolve::Resolve;
use crate::core::repository::pending_commit::{PendingCommit, PendingCommitType};
use crate::core::repository::sequencer::{Action, Sequencer};
use crate::core::revlist::RevList;
use crate::commands::commit_writer::{CommitWriter, COMMIT_NOTES};
use crate::core::workspace::Workspace;
use crate::core::revision::Revision;
use crate::core::repository::repository::Repository;

// Shared constants and utilities
const CONFLICT_NOTES: &str = "\
after resolving the conflicts, mark the corrected paths
with 'ash add <paths>' or 'ash rm <paths>'
and commit the result with 'ash commit'";

pub struct RevertCommand;

impl RevertCommand {
    pub fn execute(
        args: &[String],
        continue_op: bool,
        abort: bool,
        quit: bool,
        mainline: Option<u32>,
    ) -> Result<(), Error> {
        let root_path = Path::new(".");
        let git_path = root_path.join(".ash");
        let repo_path = git_path.clone();

        // Verify repository exists
        if !git_path.exists() {
            return Err(Error::Generic("Not an AsheraFlow repository: .ash directory not found".into()));
        }

        // Initialize repository
        let mut repo = Repository::new(".")?;
        
        // Create revert options map
        let mut options = HashMap::new();
        if let Some(mainline) = mainline {
            options.insert(String::from("mainline"), mainline.to_string());
        }

        // Initialize sequencer
        let mut sequencer = Sequencer::new(repo_path.clone());

        if continue_op {
            println!("Continuing revert operation...");
            handle_continue(root_path, repo_path, &mut repo.database, &mut repo.index, &repo.refs, &mut sequencer)?;
            return Ok(());
        } else if abort {
            println!("Aborting revert operation...");
            handle_abort(root_path, repo_path, &mut repo.database, &mut repo.index, &repo.refs, &mut sequencer, PendingCommitType::Revert)?;
            return Ok(());
        } else if quit {
            println!("Quitting revert operation without aborting...");
            handle_quit(root_path, repo_path, &mut repo.database, &mut repo.index, &repo.refs, &mut sequencer, PendingCommitType::Revert)?;
            return Ok(());
        } else {
            println!("Starting revert operation for {} commits...", args.len());
            sequencer.start(&options)?;
            
            // Get the commits to revert and add them to the sequencer
            store_commit_sequence(&mut sequencer, &mut repo, args)?;
            
            println!("Added {} commits to revert", args.len());
        }
        
        // Process the first commit
        if let Some((action, commit)) = sequencer.next_command() {
            // Initialize commit writer after revlist processing to avoid multiple mutable borrows
            let mut commit_writer = CommitWriter::new(
                root_path,
                repo_path,
                &mut repo.database,
                &mut repo.index,
                &repo.refs
            );
            
            match action {
                Action::Revert => {
                    let commit_oid = commit.get_oid().map_or_else(String::new, |s| s.clone());
                    println!("Reverting commit: {}", commit_oid);
                    
                    // Create a message for the revert
                    let message = format!(
                        "Revert \"{}\"

This reverts commit {}.",
                        commit.title_line().trim(),
                        commit_oid
                    );
                    
                    // Get the current HEAD as parent
                    let head_ref = repo.refs.read_head()?.unwrap_or_else(String::new);
                    
                    // Use CommitWriter to handle the commit creation
                    let parents = vec![head_ref];
                    let new_commit = commit_writer.write_commit(parents, &message, None)?;
                    
                    // Print commit info
                    commit_writer.print_commit(&new_commit)?;
                    
                    sequencer.drop_command()?;
                    println!("Successfully reverted commit");
                },
                Action::Pick => {
                    return Err(Error::Generic("Pick action not supported in revert".into()));
                }
            }
        }
        
        Ok(())
    }
}

fn store_commit_sequence(
    sequencer: &mut Sequencer,
    repo: &mut Repository,
    args: &[String]
) -> Result<(), Error> {
    // Resolve each commit hash separately using Revision
    let mut resolved_oids = Vec::new();
    for arg in args {
        let mut revision = Revision::new(repo, arg);
        match revision.resolve("commit") {
            Ok(oid) => {
                resolved_oids.push(oid);
            },
            Err(e) => {
                // Handle invalid revision
                for err in revision.errors {
                    eprintln!("error: {}", err.message);
                    for hint in &err.hint {
                        eprintln!("hint: {}", hint);
                    }
                }
                return Err(e);
            }
        }
    }
    
    // Get the commits using resolved OIDs
    let mut commits = Vec::new();
    for oid in resolved_oids {
        let commit_obj = repo.database.load(&oid)?;
        if let Some(commit) = commit_obj.as_any().downcast_ref::<Commit>() {
            commits.push(commit.clone());
        } else {
            return Err(Error::Generic(format!("Object {} is not a commit", oid)));
        }
    }
    
    // Add reverts in order
    for commit in commits.iter() {
        sequencer.add_revert(commit.to_owned());
    }

    Ok(())
}

fn revert(
    sequencer: &mut Sequencer,
    commit: &Commit,
    database: &mut Database,
    index: &mut Index,
    refs: &Refs,
) -> Result<(), Error> {
    // Generate merge inputs for revert
    let inputs = revert_merge_inputs(sequencer, commit, refs)?;
    let message = revert_commit_message(commit);

    // Resolve merge
    index.load_for_update()?;
    
    // Create workspace outside the borrow scope
    let workspace = Workspace::new(Path::new("."));
    {
        Resolve::new(database, &workspace, index, &inputs).execute()?;
    }
    
    index.write_updates()?;

    // Check for conflicts before creating the commit writer
    let has_conflict = index.has_conflict();

    // Create commit writer
    let root_path = Path::new(".");
    let git_path = root_path.join(".ash");
    let mut commit_writer = CommitWriter::new(
        root_path,
        git_path,
        database,
        index,
        refs
    );

    // Handle conflicts if any
    if has_conflict {
        return fail_on_conflict(
            &mut commit_writer,
            sequencer,
            &inputs,
            PendingCommitType::Revert,
            &message,
        );
    }

    // Get editor command and prepare commit message
    let editor_cmd = commit_writer.get_editor_command();
    let edited_message = edit_revert_message(&mut commit_writer, &message, editor_cmd)?;
    
    // If message editing was aborted, abort the revert
    let message = match edited_message {
        Some(msg) => msg,
        None => return Err(Error::Generic("Aborting revert due to empty commit message".into())),
    };
    
    // Get the current HEAD and author
    let head_ref = refs.read_head()?.unwrap_or_else(String::new);
    let author = commit_writer.current_author();
    
    // Use CommitWriter to create the commit
    let parents = vec![head_ref];
    let new_commit = commit_writer.write_commit(parents, &message, Some(author))?;
    
    // Print commit info
    commit_writer.print_commit(&new_commit)?;

    Ok(())
}

fn revert_merge_inputs(
    sequencer: &mut Sequencer,
    commit: &Commit,
    refs: &Refs,
) -> Result<inputs::CherryPick, Error> {
    let db_path = Path::new(".").join(".ash").join("objects");
    let database = Database::new(db_path);
    let commit_oid = commit.get_oid().map_or_else(String::new, |s| s.clone());
    let short = database.short_oid(&commit_oid);

    let left_name = HEAD.to_owned();
    let left_oid = refs.read_head()?.unwrap_or_else(String::new);

    let right_name = format!("parent of {}... {}", short, commit.title_line().trim());
    let right_oid = select_parent(sequencer, commit)?;

    Ok(inputs::CherryPick::new(
        left_name,
        right_name,
        left_oid,
        right_oid,
        vec![commit_oid],
    ))
}

fn revert_commit_message(commit: &Commit) -> String {
    let commit_oid = commit.get_oid().map_or_else(String::new, |s| s.clone());
    format!(
        "Revert \"{}\"

This reverts commit {}.
",
        commit.title_line().trim(),
        commit_oid
    )
}

fn edit_revert_message(
    commit_writer: &mut CommitWriter,
    message: &str,
    editor_cmd: String,
) -> Result<Option<String>, Error> {
    let message_path = commit_writer.commit_message_path();
    
    Editor::edit(message_path, Some(editor_cmd), |editor| {
        editor.write(message)?;
        editor.write("")?;
        editor.note(COMMIT_NOTES)?;

        Ok(())
    })
}

fn select_parent(sequencer: &mut Sequencer, commit: &Commit) -> Result<String, Error> {
    let mainline = sequencer.get_option("mainline")?;
    
    let mainline = match mainline {
        Some(value) => value.parse::<usize>().ok(),
        None => None,
    };

    // Check if commit has multiple parents (is a merge)
    let parent = commit.get_parent();
    if parent.is_none() {
        return Err(Error::Generic(format!(
            "error: commit {} has no parent",
            commit.get_oid().map_or_else(String::new, |s| s.clone())
        )));
    }

    // For now, we'll just use the one parent from get_parent()
    // In a real implementation, we'd need to load the commit object and examine all parents
    let commit_oid = commit.get_oid().map_or_else(String::new, |s| s.clone());
    
    if mainline.is_some() {
        // In a proper implementation, we'd check if this is a merge commit with multiple parents
        return Err(Error::Generic(format!(
            "error: mainline was specified but commit {} is not properly handled as a merge yet",
            commit_oid
        )));
    }
    
    // Just return the first parent
    Ok(parent.unwrap().clone())
}

fn handle_continue(
    root_path: &Path,
    repo_path: PathBuf,
    database: &mut Database,
    index: &mut Index,
    refs: &Refs,
    sequencer: &mut Sequencer,
) -> Result<(), Error> {
    index.load()?;

    {
        let mut commit_writer = CommitWriter::new(
            root_path,
            repo_path.clone(),
            database,
            index, 
            refs
        );

        if commit_writer.pending_commit.in_progress(PendingCommitType::Revert) {
            let editor_cmd = commit_writer.get_editor_command();
            if let Err(err) = commit_writer.write_revert_commit(Some(editor_cmd)) {
                return Err(Error::Generic(format!("fatal: {}", err)));
            }
        }
    }

    sequencer.load()?;
    sequencer.drop_command()?;
    resume_sequencer(sequencer, database, index, refs)?;

    Ok(())
}

fn resume_sequencer(
    sequencer: &mut Sequencer,
    database: &mut Database,
    index: &mut Index,
    refs: &Refs,
) -> Result<(), Error> {
    while let Some((action, commit)) = sequencer.next_command() {
        match action {
            Action::Pick => return Err(Error::Generic("Pick action not supported in revert".into())),
            Action::Revert => revert(sequencer, &commit, database, index, refs)?,
        }
        sequencer.drop_command()?;
    }

    sequencer.quit()?;
    Ok(())
}

fn fail_on_conflict(
    commit_writer: &mut CommitWriter,
    sequencer: &mut Sequencer,
    inputs: &inputs::CherryPick,
    merge_type: PendingCommitType,
    message: &str,
) -> Result<(), Error> {
    sequencer.dump()?;

    commit_writer
        .pending_commit
        .start(&inputs.right_oid, merge_type)?;

    let editor_command = commit_writer.get_editor_command();
    let message_path = commit_writer.pending_commit.message_path.clone();
    
    Editor::edit(message_path, Some(editor_command), |editor| {
        editor.write(message)?;
        editor.write("")?;
        editor.note("Conflicts:")?;
        for name in commit_writer.index.conflict_paths() {
            editor.note(&format!("\t{}", name))?;
        }
        editor.close();

        Ok(())
    })?;

    println!("error: could not apply {}", inputs.right_name);
    for line in CONFLICT_NOTES.lines() {
        println!("hint: {}", line);
    }

    Err(Error::Generic("Revert failed due to conflicts".into()))
}

fn handle_abort(
    root_path: &Path,
    repo_path: PathBuf,
    database: &mut Database,
    index: &mut Index,
    refs: &Refs,
    sequencer: &mut Sequencer,
    merge_type: PendingCommitType,
) -> Result<(), Error> {
    {
        let mut commit_writer = CommitWriter::new(
            root_path,
            repo_path,
            database,
            index,
            refs
        );

        if commit_writer.pending_commit.in_progress(merge_type) {
            commit_writer.pending_commit.clear(merge_type)?;
        }
    }
    
    index.load_for_update()?;

    match sequencer.abort() {
        Ok(()) => (),
        Err(err) => {
            println!("warning: {}", err);
        }
    }

    index.write_updates()?;
    
    Ok(())
}

fn handle_quit(
    root_path: &Path,
    repo_path: PathBuf,
    database: &mut Database,
    index: &mut Index,
    refs: &Refs,
    sequencer: &mut Sequencer,
    merge_type: PendingCommitType,
) -> Result<(), Error> {
    {
        let mut commit_writer = CommitWriter::new(
            root_path,
            repo_path,
            database,
            index,
            refs
        );

        if commit_writer.pending_commit.in_progress(merge_type) {
            commit_writer.pending_commit.clear(merge_type)?;
        }
    }
    
    sequencer.quit()?;

    Ok(())
} 