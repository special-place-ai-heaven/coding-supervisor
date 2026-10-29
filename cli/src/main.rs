//! `supervisor`: one entry point for the completion gate's commands.
#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;

const USAGE: &str =
    "usage: supervisor mcp [--project <dir>]  (serve the gate as an MCP server on stdin/stdout)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.split_first() {
        Some((command, rest)) if command == "mcp" => supervisor_mcp::run_stdio(rest),
        _ => {
            let _ = writeln!(io::stderr(), "{USAGE}");
            ExitCode::from(2)
        }
    }
}
