//! `node --test --test-reporter=tap`. `not ok N - NAME` lines followed by an
//! indented YAML block with `location`, `failureType`, `error`, and `stack`.
//! Nested tests are indented under a `# Subtest: NAME` line; a parent whose
//! children failed reports `failureType: 'subtestsFailed'` and is skipped.

use std::path::Path;
use std::process::Command;

use crate::model::{trim_output, Adapter, AdapterId, Failure, RerunKey, RunResult};

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
        stdout: &str,
        stderr: &str,
        code: i32,
    ) -> RunResult {
        let mut result = RunResult::new(self.id(), root, code);
        let entries = parse_tap(stdout);
        if entries.is_empty() && code != 0 {
            let text = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            result.build_error = Some(if text.trim().is_empty() {
                "node --test produced no TAP output".to_string()
            } else {
                text.trim().to_string()
            });
            return result;
        }
        for e in entries {
            if e.ok || e.yaml("failureType") == Some("subtestsFailed") {
                continue;
            }
            let (file, line) = super::first_project_frame(&e.block_text("stack"), root)
                .or_else(|| {
                    let loc = e.yaml("location")?;
                    let (rest, _col) = loc.rsplit_once(':')?;
                    let (path, line) = rest.rsplit_once(':')?;
                    Some((super::relative_to(root, path), line.parse().ok()?))
                })
                .unzip();
            let error = e.block_text("error");
            let output = if error.trim().is_empty() {
                e.yaml_lines.join("\n")
            } else {
                error
            };
            result.failures.push(Failure {
                adapter: AdapterId(self.id()),
                name: e.display_name(),
                file: file.clone(),
                line,
                output: trim_output(&output),
                rerun: RerunKey::NodeTest {
                    file: file.unwrap_or_default(),
                    name: e.name.clone(),
                },
            });
        }
        result.failed = result.failures.len() as u32;
        result.passed = trailer(stdout, "pass");
        result.skipped = trailer(stdout, "skipped") + trailer(stdout, "todo");
        result
    }
}

/// One `ok` / `not ok` line with its YAML block and ancestors.
#[derive(Debug, Default)]
struct Entry {
    ok: bool,
    name: String,
    /// Enclosing subtest names, outermost first. A file-level entry (node
    /// runs each file as a subtest named by its absolute path) is dropped.
    ancestors: Vec<String>,
    /// The YAML block's lines with the block indent removed.
    yaml_lines: Vec<String>,
}

impl Entry {
    /// A scalar from the YAML block, quotes removed.
    fn yaml(&self, key: &str) -> Option<&str> {
        let prefix = format!("{key}: ");
        self.yaml_lines
            .iter()
            .find_map(|l| l.strip_prefix(&prefix))
            .map(|v| v.trim().trim_matches('\''))
    }

    /// A `key: |-` block from the YAML, dedented. Empty when absent.
    fn block_text(&self, key: &str) -> String {
        let head = format!("{key}: |");
        let mut lines = self.yaml_lines.iter().skip_while(|l| !l.starts_with(&head));
        if lines.next().is_none() {
            return String::new();
        }
        let body: Vec<&str> = lines
            .take_while(|l| l.starts_with("  ") || l.trim().is_empty())
            .map(|l| l.strip_prefix("  ").unwrap_or(l))
            .collect();
        body.join("\n")
    }

    fn display_name(&self) -> String {
        let mut parts: Vec<&str> = self.ancestors.iter().map(String::as_str).collect();
        parts.push(&self.name);
        parts.join(" > ")
    }
}

/// Every `ok` / `not ok` entry in `tap`, in order, with nesting resolved
/// from indentation.
fn parse_tap(tap: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    // (indent, name) of open `# Subtest:` scopes.
    let mut scopes: Vec<(usize, String)> = Vec::new();
    let mut lines = tap.lines().peekable();
    while let Some(line) = lines.next() {
        let indent = line.len() - line.trim_start().len();
        let text = line.trim_start();
        if let Some(name) = text.strip_prefix("# Subtest: ") {
            scopes.push((indent, name.to_string()));
            continue;
        }
        let (ok, rest) = if let Some(r) = text.strip_prefix("not ok ") {
            (false, r)
        } else if let Some(r) = text.strip_prefix("ok ") {
            (true, r)
        } else {
            continue;
        };
        let name = rest
            .split_once(" - ")
            .map_or("", |(_, n)| n)
            .split(" # ")
            .next()
            .unwrap_or("")
            .to_string();
        // This entry closes the scope opened at the same indent.
        let popped = match scopes.last() {
            Some((i, _)) if *i == indent => scopes.pop(),
            _ => None,
        };
        let name = popped.map_or(name, |(_, n)| n);
        let ancestors = scopes
            .iter()
            .map(|(_, n)| n.clone())
            .filter(|n| !n.starts_with('/'))
            .collect();
        let mut yaml_lines = Vec::new();
        if lines.peek().is_some_and(|l| l.trim() == "---") {
            lines.next();
            let block_indent = indent + 2;
            for l in lines.by_ref() {
                if l.trim() == "..." {
                    break;
                }
                yaml_lines.push(l.get(block_indent..).unwrap_or(l.trim_start()).to_string());
            }
        }
        entries.push(Entry {
            ok,
            name,
            ancestors,
            yaml_lines,
        });
    }
    entries
}

/// `# pass 3` style trailer counts.
fn trailer(tap: &str, key: &str) -> u32 {
    let prefix = format!("# {key} ");
    tap.lines()
        .rev()
        .find_map(|l| l.strip_prefix(&prefix))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
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

    const NESTED: &str = "TAP version 13
# Subtest: /r/test/a.test.js
    # Subtest: math
        # Subtest: adds
        ok 1 - adds
          ---
          duration_ms: 1
          ...
        # Subtest: doubles
        not ok 2 - doubles
          ---
          duration_ms: 1
          location: '/r/test/a.test.js:5:3'
          failureType: 'testCodeFailure'
          error: |-
            Expected 4
            got 5
          code: 'ERR_ASSERTION'
          stack: |-
            TestContext.<anonymous> (file:///r/test/a.test.js:6:12)
            Test.run (node:internal/test_runner/test:1:1)
          ...
        1..2
    not ok 1 - math
      ---
      duration_ms: 2
      failureType: 'subtestsFailed'
      error: '1 subtest failed'
      ...
    1..1
not ok 1 - /r/test/a.test.js
  ---
  failureType: 'subtestsFailed'
  ...
1..1
# tests 1
# pass 1
# fail 1
# skipped 0
";

    #[test]
    fn nested_subtests() {
        let r = NodeTest.parse(Path::new("/r"), Path::new("/s"), NESTED, "", 1);
        assert_eq!(r.failures.len(), 1, "{:?}", r.failures);
        let f = &r.failures[0];
        assert_eq!(f.name, "math > doubles");
        assert_eq!(f.file.as_deref(), Some(Path::new("test/a.test.js")));
        assert_eq!(f.line, Some(6));
        assert_eq!(f.output, "Expected 4\ngot 5");
        assert_eq!(
            f.rerun,
            RerunKey::NodeTest {
                file: "test/a.test.js".into(),
                name: "doubles".into()
            }
        );
        assert_eq!((r.passed, r.failed), (1, 1));
    }

    #[test]
    fn location_is_the_fallback() {
        let tap = "TAP version 13\n# Subtest: t\nnot ok 1 - t\n  ---\n  location: '/r/x.test.js:3:1'\n  failureType: 'testCodeFailure'\n  error: 'boom'\n  ...\n1..1\n# pass 0\n# fail 1\n";
        let r = NodeTest.parse(Path::new("/r"), Path::new("/s"), tap, "", 1);
        let f = &r.failures[0];
        assert_eq!(f.file.as_deref(), Some(Path::new("x.test.js")));
        assert_eq!(f.line, Some(3));
        // No error block: the YAML itself is the output.
        assert!(f.output.contains("error: 'boom'"));
    }

    #[test]
    fn no_tap_and_a_bad_exit_is_a_build_error() {
        let r = NodeTest.parse(
            Path::new("/r"),
            Path::new("/s"),
            "",
            "node: bad option\n",
            9,
        );
        assert_eq!(r.build_error.as_deref(), Some("node: bad option"));
    }
}
