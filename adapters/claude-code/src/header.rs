//! Worker handback header: `VERDICT:`, `CHECKS RUN:`, then `CHECK <id> | <command> | exit <code>` lines.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckLine {
    pub criterion: String,
    pub command: String,
    pub cwd: Option<String>,
    pub exit_code: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub verdict: Verdict,
    pub checks_run: usize,
    pub checks: Vec<CheckLine>,
    pub consistent: bool,
}

/// `None` when the first two non-empty lines are not a VERDICT and a CHECKS RUN line.
/// A malformed CHECK line is dropped, which makes the header inconsistent.
pub fn parse(message: &str) -> Option<Header> {
    let mut lines = message.lines().map(str::trim).filter(|l| !l.is_empty());
    let verdict = match value(lines.next()?, "VERDICT")? {
        "PASS" => Verdict::Pass,
        "FAIL" => Verdict::Fail,
        "UNVERIFIED" => Verdict::Unverified,
        _ => return None,
    };
    let checks_run = value(lines.next()?, "CHECKS RUN")?.parse().ok()?;
    let checks: Vec<CheckLine> = lines
        .filter_map(|l| strip_key(l, "CHECK "))
        .filter_map(check_line)
        .collect();
    let consistent = checks.len() == checks_run;
    Some(Header {
        verdict,
        checks_run,
        checks,
        consistent,
    })
}

fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = line.split_once(':')?;
    k.trim().eq_ignore_ascii_case(key).then_some(v.trim())
}

fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.get(..key.len())
        .filter(|p| p.eq_ignore_ascii_case(key))?;
    line.get(key.len()..)
}

fn check_line(rest: &str) -> Option<CheckLine> {
    let mut parts = rest.split(" | ");
    let criterion = parts.next()?.trim();
    if criterion.is_empty() || criterion.contains(char::is_whitespace) {
        return None;
    }
    let (mut cwd, mut exit_code, mut command) = (None, None, Vec::new());
    for part in parts.map(str::trim) {
        if let Some(path) = strip_key(part, "cwd ") {
            if cwd.replace(path.trim().to_owned()).is_some() {
                return None;
            }
        } else if let Some(code) = strip_key(part, "exit ").and_then(|c| c.trim().parse().ok()) {
            if exit_code.replace(code).is_some() {
                return None;
            }
        } else {
            // A command that itself contains " | " keeps its pipe.
            command.push(part);
        }
    }
    let command = command.join(" | ");
    if command.is_empty() {
        return None;
    }
    Some(CheckLine {
        criterion: criterion.to_owned(),
        command,
        cwd,
        exit_code: exit_code?,
    })
}
