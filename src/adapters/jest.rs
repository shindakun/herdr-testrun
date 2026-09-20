//! `jest --json --outputFile=STATE/jest.json`. Failures are
//! `testResults[].assertionResults[]` with `status == "failed"`.

use std::path::Path;
use std::process::Command;

use crate::model::{Adapter, RerunKey, RunResult};

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
        _state: &Path,
        _stdout: &str,
        _stderr: &str,
        code: i32,
    ) -> RunResult {
        let mut r = RunResult::new(self.id(), root, code);
        r.build_error = Some(super::not_built(self.id()));
        r
    }
}

/// Distinct files and names from jest keys, in order.
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
    use crate::testutil::argv;

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
}
