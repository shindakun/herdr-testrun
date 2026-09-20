//! `cargo test --no-fail-fast`. Parses the stable libtest text format; the
//! JSON format is nightly only and not used.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::{trim_output, Adapter, AdapterId, Failure, RerunKey, RunResult};

pub struct Cargo;

impl Adapter for Cargo {
    fn id(&self) -> &'static str {
        "cargo"
    }

    fn detect(&self, root: &Path) -> bool {
        root.join("Cargo.toml").is_file()
    }

    /// Rerun passes the names as filters. libtest matches substrings, so an
    /// exact name may run extra tests; accept it.
    fn command(&self, root: &Path, _state: &Path, only: Option<&[RerunKey]>) -> Command {
        let mut cmd = super::base_command("cargo", root);
        cmd.args(["test", "--no-fail-fast"]);
        if let Some(keys) = only.filter(|k| !k.is_empty()) {
            cmd.arg("--");
            for key in keys {
                if let RerunKey::CargoTest { name } = key {
                    cmd.arg(name);
                }
            }
        }
        cmd
    }

    fn parse(
        &self,
        root: &Path,
        _state: &Path,
        stdout: &str,
        stderr: &str,
        code: i32,
    ) -> RunResult {
        let mut result = RunResult::new(self.id(), root, code);
        let mut ran_any = false;
        let mut failed_names: Vec<String> = Vec::new();
        for line in stdout.lines() {
            if line.starts_with("running ") && line.ends_with(" tests") || line == "running 1 test"
            {
                ran_any = true;
                continue;
            }
            let Some((name, outcome)) = test_line(line) else {
                continue;
            };
            match outcome {
                "ok" => result.passed += 1,
                "ignored" => result.skipped += 1,
                "FAILED" => failed_names.push(name.to_string()),
                _ => {}
            }
        }
        for name in failed_names {
            let block = stdout_block(stdout, &name).unwrap_or_default();
            let (file, line) = panic_location(&block).unzip();
            result.failures.push(Failure {
                adapter: AdapterId(self.id()),
                name: name.clone(),
                file,
                line,
                output: trim_output(&block),
                rerun: RerunKey::CargoTest { name },
            });
        }
        result.failed = result.failures.len() as u32;
        if !ran_any && code != 0 {
            result.build_error = Some(build_error(stderr));
        }
        result
    }
}

/// `test NAME ... ok` gives `(NAME, "ok")`. The outcome is the last word;
/// `test NAME - should panic ... ok` keeps `NAME`.
fn test_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("test ")?;
    let (head, outcome) = rest.rsplit_once(" ... ")?;
    let name = head.split_once(" - ").map_or(head, |(n, _)| n);
    if name.is_empty() || name.contains(' ') {
        return None;
    }
    // `FAILED` may carry a suffix such as `FAILED (exit status ...)`.
    let outcome = outcome.split_whitespace().next()?;
    Some((name, outcome))
}

/// The text between `---- NAME stdout ----` and the next `---- ` header or
/// `failures:` line, trailing blank lines and the backtrace hint removed.
fn stdout_block(stdout: &str, name: &str) -> Option<String> {
    let header = format!("---- {name} stdout ----");
    let mut lines = stdout.lines().skip_while(|l| *l != header);
    lines.next()?;
    let body: Vec<&str> = lines
        .take_while(|l| !l.starts_with("---- ") && *l != "failures:")
        .filter(|l| !l.starts_with("note: run with `RUST_BACKTRACE=1`"))
        .collect();
    let start = body
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(body.len());
    Some(body[start..].join("\n"))
}

/// `panicked at src/lib.rs:24:9:` gives the file and line.
fn panic_location(block: &str) -> Option<(PathBuf, u32)> {
    let idx = block.find("panicked at ")?;
    let rest = &block[idx + "panicked at ".len()..];
    let loc = rest.lines().next()?.trim_end_matches(':');
    let (rest, _col) = loc.rsplit_once(':')?;
    let (file, line) = rest.rsplit_once(':')?;
    Some((PathBuf::from(file), line.parse().ok()?))
}

/// Compiler output without cargo's progress lines.
fn build_error(stderr: &str) -> String {
    let text: Vec<&str> = stderr
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("Compiling ")
                || t.starts_with("Checking ")
                || t.starts_with("Updating ")
                || t.starts_with("Downloading ")
                || t.starts_with("Downloaded ")
                || t.starts_with("Finished ")
                || t.starts_with("Blocking "))
        })
        .collect();
    let text = text.join("\n").trim().to_string();
    if text.is_empty() {
        "build failed".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::argv;

    #[test]
    fn commands() {
        let s = Path::new("/s");
        assert_eq!(
            argv(&Cargo.command(Path::new("/p"), s, None)),
            ["cargo", "test", "--no-fail-fast"]
        );
        let keys = [
            RerunKey::CargoTest {
                name: "tests::roundtrip".into(),
            },
            RerunKey::CargoTest {
                name: "parse_empty".into(),
            },
        ];
        assert_eq!(
            argv(&Cargo.command(Path::new("/p"), s, Some(&keys))),
            [
                "cargo",
                "test",
                "--no-fail-fast",
                "--",
                "tests::roundtrip",
                "parse_empty"
            ]
        );
    }

    #[test]
    fn test_lines() {
        assert_eq!(
            test_line("test tests::adds ... ok"),
            Some(("tests::adds", "ok"))
        );
        assert_eq!(test_line("test a::b ... FAILED"), Some(("a::b", "FAILED")));
        assert_eq!(
            test_line("test a::b ... ignored"),
            Some(("a::b", "ignored"))
        );
        assert_eq!(
            test_line("test a::b - should panic ... ok"),
            Some(("a::b", "ok"))
        );
        assert_eq!(test_line("test result: ok. 1 passed"), None);
        assert_eq!(test_line("running 2 tests"), None);
    }

    #[test]
    fn panic_locations() {
        assert_eq!(
            panic_location("thread 'x' (123) panicked at src/lib.rs:24:9:\nassertion failed"),
            Some((PathBuf::from("src/lib.rs"), 24))
        );
        assert_eq!(
            panic_location(
                "thread 'x' panicked at tests/integration.rs:4:14:\nindex out of bounds"
            ),
            Some((PathBuf::from("tests/integration.rs"), 4))
        );
        assert_eq!(panic_location("no panic here"), None);
    }

    #[test]
    fn build_error_keeps_compiler_output() {
        let stderr = "   Compiling x v0.1.0 (/p)\nerror[E0308]: mismatched types\n --> src/lib.rs:2:5\nerror: could not compile `x` (lib test) due to 1 previous error\n";
        let r = Cargo.parse(Path::new("/p"), Path::new("/s"), "", stderr, 101);
        let err = r.build_error.unwrap();
        assert!(err.starts_with("error[E0308]"), "{err}");
        assert!(!err.contains("Compiling"));
        assert!(r.failures.is_empty());
    }

    #[test]
    fn a_clean_run_has_no_build_error() {
        let stdout = "\nrunning 1 test\ntest a ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
        let r = Cargo.parse(Path::new("/p"), Path::new("/s"), stdout, "", 0);
        assert_eq!(r.passed, 1);
        assert!(r.ok());
    }
}
