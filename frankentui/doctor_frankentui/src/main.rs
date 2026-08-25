#![forbid(unsafe_code)]

fn main() {
    if let Err(error) = doctor_frankentui::run_from_env() {
        let integration = doctor_frankentui::util::OutputIntegration::detect();
        if integration.should_emit_json() {
            eprintln!(
                "{}",
                serde_json::json!({
                    "status": "error",
                    "error": error.to_string(),
                    "exit_code": error.exit_code(),
                    "integration": integration,
                })
            );
        } else {
            eprintln!("{error}");
        }
        std::process::exit(error.exit_code());
    }
}
