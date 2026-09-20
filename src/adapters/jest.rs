//! `jest --json --outputFile=STATE/jest.json --testLocationInResults`.
//! Failures are `testResults[].assertionResults[]` with `status ==
//! "failed"`. Vitest writes the same shape and shares [`parse_report`].

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::model::{trim_output, Adapter, AdapterId, Failure, RerunKey, RunResult};

pub struct Jest;

pub const OUTPUT_FILE: &str = "jest.json";

impl Adapter for Jest {
    fn id(&self) -> &'static str {
        "jest"
    }

    fn detect(&self, root: &Path) -> bool {
        super::package_json(root).is_some_and(|p| super::has_dependency(&p, "jest"))
    }

    fn command(&self, root: &Path, state: &Path, only: Option<&[RerunKey]>) -> Command {
        let mut cmd = super::js_command(root, "jest");
        cmd.arg("--json")
            .arg(format!(
                "--outputFile={}",
                state.join(OUTPUT_FILE).display()
            ))
            .arg("--testLocationInResults");
        if let Some(keys) = only.filter(|k| !k.is_empty()) {
            let (files, names) = split_keys(keys);
            cmd.args(files);
            cmd.arg("-t").arg(name_pattern(&names));
        }
        cmd
    }

    fn parse(
        &self,
        root: &Path,
        state: &Path,
        _stdout: &str,
        stderr: &str,
        code: i32,
    ) -> RunResult {
        parse_report(
            self.id(),
            root,
            &state.join(OUTPUT_FILE),
            stderr,
            code,
            |file, name| RerunKey::JestTest { file, name },
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    #[serde(default)]
    num_passed_tests: u32,
    #[serde(default)]
    num_failed_tests: u32,
    #[serde(default)]
    num_pending_tests: u32,
    #[serde(default)]
    num_todo_tests: u32,
    #[serde(default)]
    test_results: Vec<Suite>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Suite {
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    assertion_results: Vec<Assertion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Assertion {
    full_name: String,
    status: String,
    #[serde(default)]
    failure_messages: Vec<String>,
    #[serde(default)]
    location: Option<Location>,
}

#[derive(Debug, Deserialize)]
struct Location {
    line: u32,
}

/// Parses a jest-shaped JSON report at `path`. A suite that failed with no
/// assertions (a syntax or import error) goes into `build_error`. A missing
/// report means the runner never got going; `stderr` is the build error.
pub fn parse_report(
    adapter: &'static str,
    root: &Path,
    path: &Path,
    stderr: &str,
    code: i32,
    key: impl Fn(PathBuf, String) -> RerunKey,
) -> RunResult {
    let mut result = RunResult::new(adapter, root, code);
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            result.build_error = Some(if stderr.trim().is_empty() {
                format!("{adapter} wrote no report to {}", path.display())
            } else {
                stderr.trim().to_string()
            });
            return result;
        }
    };
    let report: Report = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            result.build_error = Some(format!("{}: {e}", path.display()));
            return result;
        }
    };
    result.passed = report.num_passed_tests;
    result.skipped = report.num_pending_tests + report.num_todo_tests;
    let mut suite_errors = Vec::new();
    for suite in &report.test_results {
        let file = super::relative_to(root, &suite.name);
        if suite.status == "failed" && suite.assertion_results.is_empty() {
            let msg = suite.message.trim();
            suite_errors.push(format!(
                "{}: {}",
                file.display(),
                if msg.is_empty() { "failed to run" } else { msg }
            ));
        }
        for a in suite
            .assertion_results
            .iter()
            .filter(|a| a.status == "failed")
        {
            let message = a.failure_messages.join("\n");
            let line = match &a.location {
                Some(l) => Some(l.line),
                None => super::first_project_frame(&message, root).map(|(_, l)| l),
            };
            result.failures.push(Failure {
                adapter: AdapterId(adapter),
                name: a.full_name.clone(),
                file: Some(file.clone()),
                line,
                output: trim_output(&super::strip_noise_frames(&message)),
                rerun: key(file.clone(), a.full_name.clone()),
            });
        }
    }
    result.failed = report.num_failed_tests.max(result.failures.len() as u32);
    if !suite_errors.is_empty() {
        result.build_error = Some(suite_errors.join("\n"));
    }
    result
}

/// Distinct files and names from jest or vitest keys, in order.
pub fn split_keys(keys: &[RerunKey]) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut names = Vec::new();
    for key in keys {
        let (file, name) = match key {
            RerunKey::JestTest { file, name } | RerunKey::VitestTest { file, name } => (file, name),
            _ => continue,
        };
        let f = file.display().to_string();
        if !files.contains(&f) {
            files.push(f);
        }
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    (files, names)
}

/// `-t` takes a regex; anchor and alternate the escaped names.
pub fn name_pattern(names: &[String]) -> String {
    let parts: Vec<String> = names.iter().map(|n| regex_escape(n)).collect();
    format!("^({})$", parts.join("|"))
}

pub fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if r"\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{argv, tempdir};

    #[test]
    fn commands() {
        let root = Path::new("/p");
        let state = Path::new("/s");
        assert_eq!(
            argv(&Jest.command(root, state, None)),
            [
                "npx",
                "jest",
                "--json",
                "--outputFile=/s/jest.json",
                "--testLocationInResults"
            ]
        );
        let keys = [
            RerunKey::JestTest {
                file: "src/a.test.js".into(),
                name: "adds (1 + 2)".into(),
            },
            RerunKey::JestTest {
                file: "src/a.test.js".into(),
                name: "other".into(),
            },
        ];
        assert_eq!(
            argv(&Jest.command(root, state, Some(&keys))),
            [
                "npx",
                "jest",
                "--json",
                "--outputFile=/s/jest.json",
                "--testLocationInResults",
                "src/a.test.js",
                "-t",
                r"^(adds \(1 \+ 2\)|other)$"
            ]
        );
    }

    #[test]
    fn missing_report_is_a_build_error() {
        let state = tempdir("jest-missing");
        let r = Jest.parse(Path::new("/p"), &state, "", "Cannot find module 'x'\n", 1);
        assert_eq!(r.build_error.as_deref(), Some("Cannot find module 'x'"));
    }

    #[test]
    fn suite_that_failed_to_run_is_a_build_error() {
        let state = tempdir("jest-suite");
        std::fs::write(
            state.join(OUTPUT_FILE),
            r#"{"numFailedTests":0,"numPassedTests":0,"testResults":[{"name":"/p/broken.test.js","status":"failed","message":"SyntaxError: Unexpected token","assertionResults":[]}]}"#,
        )
        .unwrap();
        let r = Jest.parse(Path::new("/p"), &state, "", "", 1);
        assert_eq!(
            r.build_error.as_deref(),
            Some("broken.test.js: SyntaxError: Unexpected token")
        );
        assert!(r.failures.is_empty());
    }
}
