//! The `spectrum` command line: every library and editing operation, as
//! JSON. The desktop app runs it too when started as `spectrum`, so one
//! executable serves both.
mod commands;

/// Runs the command line with this process's arguments.
pub fn main() -> std::process::ExitCode {
    match commands::run() {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::json!({"ok": false, "error": format!("{error:#}")})
            );
            std::process::ExitCode::FAILURE
        }
    }
}
