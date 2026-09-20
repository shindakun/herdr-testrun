use std::process::ExitCode;

use herdr_testrun::cli::{self, USAGE};
use herdr_testrun::tui;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("pane") => tui::run(),
        Some("run") => cli::run(&args[1..]),
        Some("send") => cli::send(&args[1..]),
        Some("on-agent-idle") => cli::on_agent_idle(),
        None | Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown subcommand: {other}\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("herdr-testrun: {err}");
            ExitCode::FAILURE
        }
    }
}
