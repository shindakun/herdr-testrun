//! Spawns one test command, streams its output, and kills it on timeout.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct Output {
    pub stdout: String,
    pub stderr: String,
    /// The exit code, or -1 when the process died on a signal or timed out.
    pub code: i32,
    pub duration: Duration,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Runs `cmd` to completion or `timeout`, whichever comes first.
pub fn run(cmd: &mut Command, timeout: Duration) -> Result<Output, String> {
    run_with(cmd, timeout, |_, _| {})
}

/// Like [`run`], calling `on_line` with each line as it arrives. The child
/// gets its own process group so a timeout kills its descendants too.
pub fn run_with(
    cmd: &mut Command,
    timeout: Duration,
    mut on_line: impl FnMut(Stream, &str),
) -> Result<Output, String> {
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
    let (tx, rx) = mpsc::channel();
    let out = read_lines(child.stdout.take(), Stream::Stdout, tx.clone());
    let err = read_lines(child.stderr.take(), Stream::Stderr, tx);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut timed_out = false;
    let mut status = None;
    // Drain lines until both readers hang up; poll the child in between so a
    // timeout fires even while output is quiet.
    loop {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok((stream, line)) => {
                on_line(stream, &line);
                match stream {
                    Stream::Stdout => stdout.push_str(&line),
                    Stream::Stderr => stderr.push_str(&line),
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child.try_wait().map_err(|e| format!("wait: {e}"))?;
        }
        if status.is_none() && !timed_out && started.elapsed() >= timeout {
            timed_out = true;
            kill_group(&child);
            let _ = child.kill();
        }
    }
    let _ = out.join();
    let _ = err.join();
    let status = match status {
        Some(s) => s,
        None => child.wait().map_err(|e| format!("wait: {e}"))?,
    };
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

/// Reads `r` line by line (newlines kept) onto `tx`. Invalid UTF-8 is
/// replaced, not dropped.
fn read_lines<R: Read + Send + 'static>(
    r: Option<R>,
    stream: Stream,
    tx: mpsc::Sender<(Stream, String)>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(r) = r else { return };
        let mut reader = BufReader::new(r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx
                        .send((stream, String::from_utf8_lossy(&buf).into_owned()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
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
    fn streams_lines_in_order() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo a; echo b; printf c"]);
        let mut seen = Vec::new();
        let o = run_with(&mut cmd, Duration::from_secs(5), |s, l| {
            seen.push((s, l.to_string()))
        })
        .unwrap();
        assert_eq!(
            seen,
            vec![
                (Stream::Stdout, "a\n".to_string()),
                (Stream::Stdout, "b\n".to_string()),
                (Stream::Stdout, "c".to_string()),
            ]
        );
        assert_eq!(o.stdout, "a\nb\nc");
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
