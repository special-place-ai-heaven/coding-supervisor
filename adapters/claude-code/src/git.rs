//! Every git the adapter runs: non-interactive, without optional locks or fsmonitor, and
//! answered within a deadline so a hook never waits on git without bound.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(5);

/// Why git gave no exit status to read.
pub enum Failure {
    /// git could not be started or waited for.
    Unavailable(String),
    /// git did not finish within the deadline; the child was killed.
    TimedOut(String),
}

/// `git -C cwd args`, killed and reaped if it is still running after the deadline.
pub fn run(cwd: &Path, args: &[&str]) -> Result<Output, Failure> {
    let what = format!("git {}", args.join(" "));
    let mut command = Command::new("git");
    command
        .args(["-c", "core.fsmonitor=false", "-C"])
        .arg(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| Failure::Unavailable(format!("{what}: {e}")))?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = Instant::now();
    let timed_out =
        || Failure::TimedOut(format!("{what}: no answer within {} s", DEADLINE.as_secs()));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < DEADLINE => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(timed_out());
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Unavailable(format!("{what}: {e}")));
            }
        }
    };
    // A descendant that inherited a pipe can keep it open after git exits.
    let collect = |pipe: Receiver<Vec<u8>>| {
        pipe.recv_timeout(DEADLINE.saturating_sub(started.elapsed()))
            .map_err(|_| timed_out())
    };
    Ok(Output {
        status,
        stdout: collect(stdout)?,
        stderr: collect(stderr)?,
    })
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
    }
    receiver
}
