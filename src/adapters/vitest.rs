//! `vitest run --reporter=json --outputFile=STATE/vitest.json`. Same result
//! shape as jest.

use std::path::Path;
use std::process::Command;

use crate::model::{Adapter, RerunKey, RunResult};

pub struct Vitest;

pub const OUTPUT_FILE: &str = "vitest.json";

impl Adapter for Vitest {
    fn id(&self) -> &'static str {
        "vitest"
    }

    fn detect(&self, root: &Path) -> bool {
        super::package_json(root).is_some_and(|p| super::has_dependency(&p, "vitest"))
    }

    fn command(&self, root: &Path, state: &Path, only: Option<&[RerunKey]>) -> Command {
        let mut cmd = super::js_command(root, "vitest");
        cmd.args(["run", "--reporter=json"]).arg(format!(
            "--outputFile={}",
            state.join(OUTPUT_FILE).display()
        ));
        if let Some(keys) = only.filter(|k| !k.is_empty()) {
            let (files, names) = super::jest::split_keys(keys);
            cmd.args(files);
            cmd.arg("-t").arg(super::jest::name_pattern(&names));
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
            argv(&Vitest.command(root, state, None)),
            [
                "npx",
                "vitest",
                "run",
                "--reporter=json",
                "--outputFile=/s/vitest.json"
            ]
        );
        let keys = [RerunKey::VitestTest {
            file: "a.test.ts".into(),
            name: "adds".into(),
        }];
        assert_eq!(
            argv(&Vitest.command(root, state, Some(&keys))),
            [
                "npx",
                "vitest",
                "run",
                "--reporter=json",
                "--outputFile=/s/vitest.json",
                "a.test.ts",
                "-t",
                "^(adds)$"
            ]
        );
    }
}
