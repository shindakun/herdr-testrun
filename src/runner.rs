//! Spawns one test command, captures its output, and kills it on timeout.
//! The one-at-a-time queue and streaming to the pane land with the TUI.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Output {
    pub stdout: String,
    pub stderr: String,
    /// The exit code, or -1 when the process died on a signal or timed out.
    pub code: i32,
    pub duration: Duration,
    pub timed_out: bool,
}

/// Runs `cmd` to completion or `timeout`, whichever comes first. The child
/// gets its own process group so a timeout kills its descendants too.
pub fn run(cmd: &mut Command, timeout: Duration) -> Result<Output, String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", cmd.get_program().to_string_lossy()))?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let mut timed_out = false;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| format!("wait: {e}"))? {
            break s;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            kill_group(&child);
            let _ = child.kill();
            break child.wait().map_err(|e| format!("wait: {e}"))?;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = out.join().map_err(|_| "stdout reader panicked")?;
    let stderr = err.join().map_err(|_| "stderr reader panicked")?;
    Ok(Output {
        stdout,
        stderr,
        code: if timed_out {
            -1
        } else {
            status.code().unwrap_or(-1)
        },
        duration: started.elapsed(),
        timed_out,
    })
}

fn drain<R: Read + Send + 'static>(r: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            let _ = r.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

#[cfg(unix)]
fn kill_group(child: &std::process::Child) {
    // The child is its own group leader, so its pid is the pgid.
    // SAFETY: killpg has no memory-safety preconditions; a stale pid is a
    // harmless ESRCH.
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_child: &std::process::Child) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_and_code() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo out; echo err >&2; exit 3"]);
        let o = run(&mut cmd, Duration::from_secs(5)).unwrap();
        assert_eq!(o.stdout, "out\n");
        assert_eq!(o.stderr, "err\n");
        assert_eq!(o.code, 3);
        assert!(!o.timed_out);
    }

    #[test]
    fn times_out_and_kills_the_group() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30 & wait"]);
        let o = run(&mut cmd, Duration::from_millis(200)).unwrap();
        assert!(o.timed_out);
        assert_eq!(o.code, -1);
        assert!(o.duration < Duration::from_secs(5));
    }

    #[test]
    fn missing_program_is_an_error() {
        let mut cmd = Command::new("herdr-testrun-no-such-program");
        assert!(run(&mut cmd, Duration::from_secs(1)).is_err());
    }
}
