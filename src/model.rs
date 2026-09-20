//! The types every adapter produces and the trait every adapter implements.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Deserializer, Serialize};

/// An adapter's registry id. Deserializing looks the string up in the
/// registry, so a `last.json` written by an adapter that no longer exists
/// is an error rather than a dangling id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AdapterId(pub &'static str);

impl std::ops::Deref for AdapterId {
    type Target = str;
    fn deref(&self) -> &str {
        self.0
    }
}

impl std::fmt::Display for AdapterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl PartialEq<str> for AdapterId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AdapterId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for AdapterId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        crate::adapters::intern(&s)
            .map(AdapterId)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown adapter {s}")))
    }
}

/// Lines of runner output kept per failure. The rest is in `raw.log`.
pub const MAX_OUTPUT_LINES: usize = 40;

/// What an adapter needs to rerun one test. Only the adapter that made a key
/// reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RerunKey {
    GoTest { pkg: String, name: String },
    CargoTest { name: String },
    JestTest { file: PathBuf, name: String },
    VitestTest { file: PathBuf, name: String },
    NodeTest { file: PathBuf, name: String },
    PytestNode { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub adapter: AdapterId,
    /// `TestFoo/sub`, `tests::foo`, `adds two numbers`.
    pub name: String,
    /// Relative to the project root.
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    /// Trimmed to [`MAX_OUTPUT_LINES`].
    pub output: String,
    pub rerun: RerunKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub adapter: AdapterId,
    pub root: PathBuf,
    pub started: SystemTime,
    pub duration: Duration,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub failures: Vec<Failure>,
    pub exit_code: i32,
    /// Full stdout and stderr on disk, for the expand view.
    pub raw_log: PathBuf,
    /// Set when the runner never got to the tests.
    pub build_error: Option<String>,
}

impl RunResult {
    /// An empty result for `adapter` at `root`; the parser fills it in.
    pub fn new(adapter: &'static str, root: &Path, exit_code: i32) -> Self {
        Self {
            adapter: AdapterId(adapter),
            root: root.to_path_buf(),
            started: SystemTime::now(),
            duration: Duration::ZERO,
            passed: 0,
            failed: 0,
            skipped: 0,
            failures: Vec::new(),
            exit_code,
            raw_log: PathBuf::new(),
            build_error: None,
        }
    }

    pub fn ok(&self) -> bool {
        self.failures.is_empty() && self.build_error.is_none() && self.exit_code == 0
    }
}

pub trait Adapter: Sync {
    /// `"go"`, `"cargo"`, ...; the same string as in the config file.
    fn id(&self) -> &'static str;
    /// Whether `root` looks like a project this adapter runs.
    fn detect(&self, root: &Path) -> bool;
    /// The command for a full run, or for `only` when it is `Some`. The
    /// command's cwd is `root`. `state` is a directory the adapter may write
    /// result files into (`jest.json`, `junit.xml`); it exists.
    fn command(&self, root: &Path, state: &Path, only: Option<&[RerunKey]>) -> Command;
    /// Parses one finished run. `stdout` and `stderr` are complete; `state`
    /// is the same directory `command` was given.
    fn parse(&self, root: &Path, state: &Path, stdout: &str, stderr: &str, code: i32) -> RunResult;
}

/// The first [`MAX_OUTPUT_LINES`] of `text`, trailing blank lines removed,
/// with a marker when lines were cut.
pub fn trim_output(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(0, |i| i + 1);
    let lines = &lines[..end];
    if lines.len() <= MAX_OUTPUT_LINES {
        return lines.join("\n");
    }
    let mut out = lines[..MAX_OUTPUT_LINES].join("\n");
    out.push_str(&format!(
        "\n... {} more lines in raw.log",
        lines.len() - MAX_OUTPUT_LINES
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_short_output() {
        assert_eq!(trim_output("a\nb\n\n\n"), "a\nb");
    }

    #[test]
    fn trim_cuts_long_output() {
        let long: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        let out = trim_output(&long.join("\n"));
        assert_eq!(out.lines().count(), MAX_OUTPUT_LINES + 1);
        assert!(out.ends_with("... 10 more lines in raw.log"));
    }

    #[test]
    fn rerun_key_round_trips() {
        let k = RerunKey::GoTest {
            pkg: "example.com/m/parse".into(),
            name: "TestParse/empty".into(),
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(s.contains("\"kind\":\"go_test\""));
        assert_eq!(serde_json::from_str::<RerunKey>(&s).unwrap(), k);
    }

    #[test]
    fn unknown_adapter_id_is_an_error() {
        let json = r#"{"adapter":"nope","name":"t","file":null,"line":null,"output":"","rerun":{"kind":"cargo_test","name":"t"}}"#;
        assert!(serde_json::from_str::<Failure>(json).is_err());
    }
}
