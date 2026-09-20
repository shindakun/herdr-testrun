//! `pytest -o junit_family=xunit1 --junitxml=STATE/junit.xml -q`. Failures
//! are `<testcase>` elements with a `<failure>` or `<error>` child. The
//! default `xunit2` family drops the `file` and `line` attributes, so the
//! command forces `xunit1`; its `line` is 0-based.

use std::path::Path;
use std::process::Command;

use crate::model::{Adapter, RerunKey, RunResult};

pub struct Pytest;

pub const OUTPUT_FILE: &str = "junit.xml";

impl Adapter for Pytest {
    fn id(&self) -> &'static str {
        "pytest"
    }

    /// `pyproject.toml`, `pytest.ini`, or a `setup.cfg` with `[tool:pytest]`.
    fn detect(&self, root: &Path) -> bool {
        if root.join("pytest.ini").is_file() || root.join("pyproject.toml").is_file() {
            return true;
        }
        std::fs::read_to_string(root.join("setup.cfg"))
            .is_ok_and(|t| t.lines().any(|l| l.trim() == "[tool:pytest]"))
    }

    fn command(&self, root: &Path, state: &Path, only: Option<&[RerunKey]>) -> Command {
        let mut cmd = super::base_command("pytest", root);
        cmd.args(["-o", "junit_family=xunit1"])
            .arg(format!("--junitxml={}", state.join(OUTPUT_FILE).display()))
            .arg("-q");
        if let Some(keys) = only.filter(|k| !k.is_empty()) {
            for key in keys {
                if let RerunKey::PytestNode { id } = key {
                    cmd.arg(id);
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::argv;

    #[test]
    fn commands() {
        let root = Path::new("/p");
        let state = Path::new("/s");
        assert_eq!(
            argv(&Pytest.command(root, state, None)),
            [
                "pytest",
                "-o",
                "junit_family=xunit1",
                "--junitxml=/s/junit.xml",
                "-q"
            ]
        );
        let keys = [RerunKey::PytestNode {
            id: "tests/test_a.py::test_add".into(),
        }];
        assert_eq!(
            argv(&Pytest.command(root, state, Some(&keys))),
            [
                "pytest",
                "-o",
                "junit_family=xunit1",
                "--junitxml=/s/junit.xml",
                "-q",
                "tests/test_a.py::test_add"
            ]
        );
    }

    #[test]
    fn detect_markers() {
        let dir = crate::testutil::tempdir("pytest-detect");
        assert!(!Pytest.detect(&dir));
        std::fs::write(dir.join("setup.cfg"), "[flake8]\n").unwrap();
        assert!(!Pytest.detect(&dir));
        std::fs::write(dir.join("setup.cfg"), "[tool:pytest]\n").unwrap();
        assert!(Pytest.detect(&dir));
    }
}
