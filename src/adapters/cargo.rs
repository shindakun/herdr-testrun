//! `cargo test --no-fail-fast`. Parses the stable libtest text format; the
//! JSON format is nightly only and not used.

use std::path::Path;
use std::process::Command;

use crate::model::{Adapter, RerunKey, RunResult};

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
}
