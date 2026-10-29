use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use supervisor_claude_code::adapter::{fail_open, run_hook};
use supervisor_claude_code::measure;

const USAGE: &str = "usage: supervisor-claude-code hook  (one hook payload on stdin, at most one decision on stdout)\n       supervisor-claude-code measure [--json] <path>...  (gate-derived numbers from journals)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") if args.len() == 1 => hook(),
        Some("measure") => measure_command(&args),
        _ => {
            let _ = writeln!(io::stderr(), "{USAGE}");
            ExitCode::from(2)
        }
    }
}

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

fn measure_command(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let paths: Vec<PathBuf> = args
        .iter()
        .skip(1)
        .filter(|a| *a != "--json")
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        let _ = writeln!(io::stderr(), "{USAGE}");
        return ExitCode::from(2);
    }
    let report = measure::collect(&paths);
    for unreadable in &report.unreadable {
        let _ = writeln!(io::stderr(), "unreadable, skipped: {unreadable}");
    }
    if report.files_read.is_empty() {
        let _ = writeln!(io::stderr(), "no journal could be read");
        return ExitCode::from(1);
    }
    let out = if json {
        report.to_json().to_string()
    } else {
        report.markdown()
    };
    let _ = writeln!(io::stdout(), "{out}");
    ExitCode::SUCCESS
}
