use std::io::{self, Read, Write};
use std::process::ExitCode;

use supervisor_claude_code::adapter::{fail_open, run_hook};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args != ["hook"] {
        let _ = writeln!(
            io::stderr(),
            "usage: supervisor-claude-code hook  (one hook payload on stdin, at most one decision on stdout)"
        );
        return ExitCode::from(2);
    }
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
