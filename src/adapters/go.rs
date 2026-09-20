//! `go test -json ./...`. One JSON event per line; `Action=fail` with `Test`
//! set is a failure and that test's `output` events are its message.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::model::{trim_output, Adapter, AdapterId, Failure, RerunKey, RunResult};

pub struct Go;

#[derive(Debug, Deserialize)]
struct Event {
    #[serde(rename = "Action")]
    action: String,
    #[serde(rename = "Package", default)]
    package: String,
    #[serde(rename = "Test", default)]
    test: Option<String>,
    #[serde(rename = "Output", default)]
    output: Option<String>,
}

impl Adapter for Go {
    fn id(&self) -> &'static str {
        "go"
    }

    fn detect(&self, root: &Path) -> bool {
        root.join("go.mod").is_file()
    }

    /// Full run: `go test -json ./...`. Rerun: the failed tests' packages with
    /// `-run` anchored on their top-level names. `-run` cannot select one
    /// subtest per parent across several parents in one regex, so a failing
    /// subtest reruns its whole parent; accept it.
    fn command(&self, root: &Path, _state: &Path, only: Option<&[RerunKey]>) -> Command {
        let mut cmd = super::base_command("go", root);
        cmd.args(["test", "-json"]);
        let Some(keys) = only.filter(|k| !k.is_empty()) else {
            cmd.arg("./...");
            return cmd;
        };
        let mut pkgs: Vec<&str> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for key in keys {
            let RerunKey::GoTest { pkg, name } = key else {
                continue;
            };
            if !pkgs.contains(&pkg.as_str()) {
                pkgs.push(pkg);
            }
            let top = regex_escape(name.split('/').next().unwrap_or(name));
            if !names.contains(&top) {
                names.push(top);
            }
        }
        cmd.args(pkgs);
        cmd.arg("-run").arg(format!("^({})$", names.join("|")));
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
        let module = module_path(root);
        let mut result = RunResult::new(self.id(), root, code);
        // Output lines per (package, test), in arrival order.
        let mut outputs: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        let mut failed: Vec<(String, String)> = Vec::new();
        let mut build_output = String::new();
        let mut build_failed = false;
        for line in stdout.lines() {
            let Ok(ev) = serde_json::from_str::<Event>(line) else {
                continue;
            };
            match ev.action.as_str() {
                "build-output" => build_output.push_str(ev.output.as_deref().unwrap_or("")),
                "build-fail" => build_failed = true,
                "output" => {
                    let out = ev.output.unwrap_or_default();
                    match ev.test {
                        Some(t) => outputs.entry((ev.package, t)).or_default().push(out),
                        None if out.contains("[build failed]")
                            || out.contains("[setup failed]") =>
                        {
                            build_failed = true;
                        }
                        None => {}
                    }
                }
                "pass" if ev.test.is_some() => result.passed += 1,
                "skip" if ev.test.is_some() => result.skipped += 1,
                "fail" => {
                    if let Some(t) = ev.test {
                        failed.push((ev.package, t));
                    }
                }
                _ => {}
            }
        }
        // A parent fails whenever a subtest fails; list only the leaves.
        let leaves: Vec<&(String, String)> = failed
            .iter()
            .filter(|(pkg, name)| {
                !failed.iter().any(|(p, n)| {
                    p == pkg && n.len() > name.len() && n.starts_with(&format!("{name}/"))
                })
            })
            .collect();
        for (pkg, name) in leaves {
            let lines: Vec<&str> = outputs
                .get(&(pkg.clone(), name.clone()))
                .map(|v| v.iter().map(String::as_str).collect())
                .unwrap_or_default();
            let body: String = lines.iter().filter(|l| !is_frame(l)).copied().collect();
            let (file, line) = lines.iter().find_map(|l| file_line(l)).unzip();
            let file = file.map(|f| package_dir(&module, pkg).join(f));
            result.failures.push(Failure {
                adapter: AdapterId(self.id()),
                name: name.clone(),
                file,
                line,
                output: trim_output(&body),
                rerun: RerunKey::GoTest {
                    pkg: pkg.clone(),
                    name: name.clone(),
                },
            });
        }
        result.failed = result.failures.len() as u32;
        if build_failed || !build_output.is_empty() {
            let text = if build_output.is_empty() {
                stderr.trim().to_string()
            } else {
                build_output.trim_end().to_string()
            };
            result.build_error = Some(if text.is_empty() {
                "build failed".to_string()
            } else {
                text
            });
        }
        result
    }
}

/// `=== RUN`, `=== PAUSE`, `=== CONT`, and `--- FAIL:` lines frame the real
/// output and carry nothing the list does not already show.
fn is_frame(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with("=== ") || l.starts_with("--- FAIL") || l.starts_with("--- PASS")
}

/// `    file_test.go:42: message` gives the file and line.
fn file_line(line: &str) -> Option<(PathBuf, u32)> {
    let l = line.trim_start();
    let (file, rest) = l.split_once(':')?;
    if !file.ends_with(".go") || file.contains(' ') {
        return None;
    }
    let (num, _) = rest.split_once(':')?;
    Some((PathBuf::from(file), num.parse().ok()?))
}

/// The `module` line of `go.mod`, or empty when it is unreadable.
fn module_path(root: &Path) -> String {
    std::fs::read_to_string(root.join("go.mod"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("module ").map(|m| m.trim().to_string()))
        })
        .unwrap_or_default()
}

/// The directory of import path `pkg` relative to the module root.
fn package_dir(module: &str, pkg: &str) -> PathBuf {
    if pkg == module {
        return PathBuf::new();
    }
    match pkg.strip_prefix(module).and_then(|r| r.strip_prefix('/')) {
        Some(rel) => PathBuf::from(rel),
        None => PathBuf::from(pkg),
    }
}

fn regex_escape(s: &str) -> String {
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
    fn full_command() {
        let cmd = Go.command(Path::new("/p"), Path::new("/s"), None);
        assert_eq!(argv(&cmd), ["go", "test", "-json", "./..."]);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/p")));
    }

    #[test]
    fn rerun_groups_packages_and_anchors_top_level_names() {
        let keys = [
            RerunKey::GoTest {
                pkg: "m/a".into(),
                name: "TestParse/empty_input".into(),
            },
            RerunKey::GoTest {
                pkg: "m/a".into(),
                name: "TestFirstFails".into(),
            },
            RerunKey::GoTest {
                pkg: "m/b".into(),
                name: "TestX.Y".into(),
            },
        ];
        let cmd = Go.command(Path::new("/p"), Path::new("/s"), Some(&keys));
        assert_eq!(
            argv(&cmd),
            [
                "go",
                "test",
                "-json",
                "m/a",
                "m/b",
                "-run",
                r"^(TestParse|TestFirstFails|TestX\.Y)$"
            ]
        );
    }

    #[test]
    fn empty_rerun_is_a_full_run() {
        assert_eq!(
            argv(&Go.command(Path::new("/p"), Path::new("/s"), Some(&[]))),
            ["go", "test", "-json", "./..."]
        );
    }

    #[test]
    fn file_line_parses_test_output() {
        assert_eq!(
            file_line("    parse_test.go:25: got \"\", want \"x\"\n"),
            Some((PathBuf::from("parse_test.go"), 25))
        );
        assert_eq!(file_line("=== RUN   TestX"), None);
        assert_eq!(file_line("    some text: not a file"), None);
    }

    #[test]
    fn package_dir_strips_module() {
        assert_eq!(
            package_dir("m", "m/internal/parse"),
            PathBuf::from("internal/parse")
        );
        assert_eq!(package_dir("m", "m"), PathBuf::new());
        assert_eq!(package_dir("", "other/pkg"), PathBuf::from("other/pkg"));
    }

    #[test]
    fn detect_needs_go_mod() {
        let dir = crate::testutil::tempdir("go-detect");
        assert!(!Go.detect(&dir));
        std::fs::write(dir.join("go.mod"), "module x\n").unwrap();
        assert!(Go.detect(&dir));
    }
}
