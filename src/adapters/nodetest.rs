//! `node --test --test-reporter=tap`. `not ok N - NAME` lines followed by an
//! indented YAML block with `failureType`, `error`, and `stack`.

use std::path::Path;
use std::process::Command;

use crate::model::{Adapter, RerunKey, RunResult};

pub struct NodeTest;

impl Adapter for NodeTest {
    fn id(&self) -> &'static str {
        "nodetest"
    }

    /// `scripts.test` starts with `node --test`.
    fn detect(&self, root: &Path) -> bool {
        super::package_json(root)
            .and_then(|p| p["scripts"]["test"].as_str().map(str::to_string))
            .is_some_and(|t| t.trim_start().starts_with("node --test"))
    }

    fn command(&self, root: &Path, _state: &Path, only: Option<&[RerunKey]>) -> Command {
        let mut cmd = super::base_command("node", root);
        cmd.args(["--test", "--test-reporter=tap"]);
        if let Some(keys) = only.filter(|k| !k.is_empty()) {
            let mut files: Vec<String> = Vec::new();
            for key in keys {
                let RerunKey::NodeTest { file, name } = key else {
                    continue;
                };
                cmd.arg(format!(
                    "--test-name-pattern=^{}$",
                    super::jest::regex_escape(name)
                ));
                let f = file.display().to_string();
                if !files.contains(&f) {
                    files.push(f);
                }
            }
            cmd.args(files);
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
            argv(&NodeTest.command(root, state, None)),
            ["node", "--test", "--test-reporter=tap"]
        );
        let keys = [RerunKey::NodeTest {
            file: "test/a.test.js".into(),
            name: "adds".into(),
        }];
        assert_eq!(
            argv(&NodeTest.command(root, state, Some(&keys))),
            [
                "node",
                "--test",
                "--test-reporter=tap",
                "--test-name-pattern=^adds$",
                "test/a.test.js"
            ]
        );
    }

    #[test]
    fn detect_reads_the_test_script() {
        let dir = crate::testutil::tempdir("nodetest-detect");
        std::fs::write(dir.join("package.json"), r#"{"scripts":{"test":"jest"}}"#).unwrap();
        assert!(!NodeTest.detect(&dir));
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"test":"node --test"}}"#,
        )
        .unwrap();
        assert!(NodeTest.detect(&dir));
    }
}
