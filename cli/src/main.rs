//! `supervisor`: one entry point for the completion gate's commands.
#![forbid(unsafe_code)]

mod init;
mod install;
mod status;
mod task;

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use supervisor_claude_code::adapter::{fail_open, run_hook};

const USAGE: &str = "usage: supervisor mcp [--project <dir>]  (serve the gate as an MCP server on stdin/stdout)
       supervisor hook  (one Claude Code hook payload on stdin, at most one decision on stdout)
       supervisor init [--project <dir>] [--output <file>]  (write an inactive editable draft contract)
       supervisor task <start|next> --contract <file> [--project <dir>] [--task <active-id>] [--disposition <cancelled|failed|blocked|budget_exhausted>]
       supervisor task <resume|deactivate> --task <active-id> [--project <dir>] [--disposition <outcome>]
       supervisor task rebind --task <active-id> --session <session-id> [--project <dir>]
       supervisor task migrate --legacy <directory> [--project <dir>]
       supervisor status [--project <dir>] [--harness <claude-code|codex>] [--json]
       supervisor install --harness <claude-code|codex|cursor|omp|grok> [--project <dir>] [--home <dir>] [--binary <path>] [--dry-run]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.split_first() {
        Some((command, rest)) if command == "mcp" => return supervisor_mcp::run_stdio(rest),
        Some((command, rest)) if command == "hook" && rest.is_empty() => return hook(),
        // Asked-for help is not a usage error (the npm launcher passes the exit code through).
        Some((command, [])) if command == "--help" || command == "-h" => {
            let _ = writeln!(io::stdout(), "{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some((command, rest)) if command == "init" => {
            parse_flags(rest, &["--project", "--output"], &[]).and_then(|f| init::run(&f))
        }
        Some((command, rest)) if command == "task" => match rest.split_first() {
            Some((action, rest)) => parse_flags(
                rest,
                &[
                    "--project",
                    "--contract",
                    "--task",
                    "--disposition",
                    "--session",
                    "--legacy",
                ],
                &[],
            )
            .and_then(|f| task::run(action, &f)),
            None => Err(Failure::Usage("task requires a command".to_owned())),
        },
        Some((command, rest)) if command == "status" => {
            parse_flags(rest, &["--project", "--harness"], &["--json"])
                .and_then(|f| status::run(&f))
        }
        Some((command, rest)) if command == "install" => parse_flags(
            rest,
            &["--harness", "--project", "--home", "--binary"],
            &["--dry-run"],
        )
        .and_then(|f| install::run(&f)),
        _ => Err(Failure::Usage(String::new())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(why)) => {
            if !why.is_empty() {
                let _ = writeln!(io::stderr(), "{why}");
            }
            let _ = writeln!(io::stderr(), "{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Error(why)) => {
            let _ = writeln!(io::stderr(), "error: {why}");
            ExitCode::from(1)
        }
    }
}

/// Same behaviour as `supervisor-claude-code hook`.
fn hook() -> ExitCode {
    let mut input = String::new();
    let output = match io::stdin().read_to_string(&mut input) {
        Ok(_) => run_hook(&input),
        Err(e) => fail_open(&format!("stdin not read ({e}); not journaled"))
            .to_json()
            .map(|v| v.to_string()),
    };
    if let Some(line) = output {
        let _ = writeln!(io::stdout(), "{line}");
    }
    ExitCode::SUCCESS
}

pub enum Failure {
    /// Bad arguments: exit 2 with the usage text.
    Usage(String),
    /// The command could not do its work: exit 1, nothing further written.
    Error(String),
}

/// Parsed `--name value` options and bare `--switch` flags.
pub struct Flags {
    values: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Flags {
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
    pub fn switch(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
    /// `--project`, else the current directory, made absolute.
    pub fn project(&self) -> Result<PathBuf, Failure> {
        let dir = match self.value("--project") {
            Some(dir) => PathBuf::from(dir),
            None => std::env::current_dir()
                .map_err(|e| Failure::Error(format!("current directory: {e}")))?,
        };
        std::path::absolute(&dir).map_err(|e| Failure::Error(format!("{}: {e}", dir.display())))
    }
}

fn parse_flags(args: &[String], valued: &[&str], switches: &[&str]) -> Result<Flags, Failure> {
    let mut flags = Flags {
        values: Vec::new(),
        switches: Vec::new(),
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if switches.contains(&arg.as_str()) {
            flags.switches.push(arg.clone());
        } else if valued.contains(&arg.as_str()) {
            let value = rest
                .next()
                .ok_or_else(|| Failure::Usage(format!("{arg} needs a value")))?;
            flags.values.push((arg.clone(), value.clone()));
        } else {
            return Err(Failure::Usage(format!("unknown argument: {arg}")));
        }
    }
    Ok(flags)
}
