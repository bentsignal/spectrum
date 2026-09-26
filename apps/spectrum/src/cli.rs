mod commands;

fn main() -> std::process::ExitCode {
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
