use workspace_guardrails::project_docs::{Task, check, read_tasks, workspace_root};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "list".into());
    let root = workspace_root();
    match command.as_str() {
        "list" => {
            check(&root)?;
            for Task {
                status,
                priority,
                title,
                path,
            } in read_tasks(&root)?
            {
                if status != "done" && status != "canceled" {
                    println!("{priority:7} {status:11} {title} ({path})");
                }
            }
        }
        "check" => {
            check(&root)?;
            println!("Project documentation and tasks are valid.");
        }
        _ => return Err(format!("unknown command: {command}; use list or check").into()),
    }
    Ok(())
}
