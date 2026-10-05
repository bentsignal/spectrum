use anyhow::Result;

use super::{Command, Project, Workspace};

pub(crate) fn apply_replay_command(project: &mut Project, command: Command) -> Result<()> {
    let before = project.clone();
    let mut workspace = Workspace::new(std::mem::take(project), None);
    workspace.legacy_photo_history = false;
    match workspace.execute_in_memory(command) {
        Ok(_) => {
            *project = workspace.project;
            Ok(())
        }
        Err(error) => {
            *project = before;
            Err(error)
        }
    }
}
