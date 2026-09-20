//! `pytest -o junit_family=xunit1 --junitxml=STATE/junit.xml -q`. Failures
//! are `<testcase>` elements with a `<failure>` or `<error>` child. The
//! default `xunit2` family drops the `file` and `line` attributes, so the
//! command forces `xunit1`; its `line` is 0-based.

use std::path::{Path, PathBuf};
use std::process::Command;

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use crate::model::{trim_output, Adapter, AdapterId, Failure, RerunKey, RunResult};

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

    fn parse(&self, root: &Path, state: &Path, stdout: &str, stderr: &str, code: i32) -> RunResult {
        let mut result = RunResult::new(self.id(), root, code);
        let path = state.join(OUTPUT_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                let text = if stderr.trim().is_empty() {
                    stdout
                } else {
                    stderr
                };
                result.build_error = Some(if text.trim().is_empty() {
                    format!("pytest wrote no report to {}", path.display())
                } else {
                    text.trim().to_string()
                });
                return result;
            }
        };
        let report = match parse_junit(&text) {
            Ok(r) => r,
            Err(e) => {
                result.build_error = Some(format!("{}: {e}", path.display()));
                return result;
            }
        };
        result.passed = report.passed;
        result.skipped = report.skipped;
        for case in report.cases {
            let Some(problem) = case.problem else {
                continue;
            };
            let id = node_id(&case.file, &case.classname, &case.name);
            let mut output = problem.text.trim_end().to_string();
            if problem.kind == "error" && !problem.message.is_empty() {
                output = format!("{}\n{output}", problem.message);
            }
            result.failures.push(Failure {
                adapter: AdapterId(self.id()),
                name: id.clone(),
                file: (!case.file.is_empty()).then(|| PathBuf::from(&case.file)),
                line: case.line.map(|l| l + 1),
                output: trim_output(&output),
                rerun: RerunKey::PytestNode { id },
            });
        }
        result.failed = result.failures.len() as u32;
        result
    }
}

#[derive(Debug, Default)]
struct Junit {
    passed: u32,
    skipped: u32,
    cases: Vec<Case>,
}

#[derive(Debug, Default)]
struct Case {
    classname: String,
    name: String,
    file: String,
    line: Option<u32>,
    problem: Option<Problem>,
}

#[derive(Debug, Default)]
struct Problem {
    /// `failure` or `error`.
    kind: String,
    message: String,
    text: String,
}

fn parse_junit(xml: &str) -> Result<Junit, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut out = Junit::default();
    let mut case: Option<Case> = None;
    let mut problem: Option<Problem> = None;
    let mut skipped = false;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            // An empty <testcase/> is a pass with nothing inside.
            Event::Empty(e) if e.name().as_ref() == b"testcase" => {
                finish_case(&mut out, testcase(&e), false);
            }
            Event::Start(e) if e.name().as_ref() == b"testcase" => {
                skipped = false;
                case = Some(testcase(&e));
            }
            Event::End(e) if e.name().as_ref() == b"testcase" => {
                if let Some(c) = case.take() {
                    finish_case(&mut out, c, skipped);
                }
            }
            Event::Start(e) | Event::Empty(e)
                if matches!(e.name().as_ref(), b"failure" | b"error") =>
            {
                problem = Some(Problem {
                    kind: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    message: attr(&e, "message"),
                    text: String::new(),
                });
            }
            Event::End(e) if matches!(e.name().as_ref(), b"failure" | b"error") => {
                if let (Some(c), Some(p)) = (case.as_mut(), problem.take()) {
                    c.problem = Some(p);
                }
            }
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"skipped" => {
                skipped = true;
            }
            Event::Text(t) => {
                if let Some(p) = problem.as_mut() {
                    p.text
                        .push_str(&t.decode().map_err(|e| e.to_string())?.replace("\r\n", "\n"));
                }
            }
            Event::CData(t) => {
                if let Some(p) = problem.as_mut() {
                    p.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            // `&gt;` and `&#10;` arrive as their own events.
            Event::GeneralRef(r) => {
                if let Some(p) = problem.as_mut() {
                    let name = r.decode().map_err(|e| e.to_string())?;
                    match r.resolve_char_ref().map_err(|e| e.to_string())? {
                        Some(c) => p.text.push(c),
                        None => p
                            .text
                            .push_str(resolve_predefined_entity(&name).unwrap_or("")),
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn testcase(e: &quick_xml::events::BytesStart) -> Case {
    Case {
        classname: attr(e, "classname"),
        name: attr(e, "name"),
        file: attr(e, "file"),
        line: attr(e, "line").parse().ok(),
        problem: None,
    }
}

/// An attribute's unescaped value, or empty.
fn attr(e: &quick_xml::events::BytesStart, name: &str) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name.as_bytes())
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
        .unwrap_or_default()
}

fn finish_case(out: &mut Junit, c: Case, skipped: bool) {
    if skipped {
        out.skipped += 1;
    } else if c.problem.is_none() {
        out.passed += 1;
    }
    out.cases.push(c);
}

/// `tests/test_a.py::TestX::test_y` from the junit attributes. `classname`
/// is the module path plus any class path; the module part repeats `file`.
fn node_id(file: &str, classname: &str, name: &str) -> String {
    if file.is_empty() {
        return if classname.is_empty() {
            name.to_string()
        } else {
            format!("{classname}::{name}")
        };
    }
    let module = file
        .strip_suffix(".py")
        .unwrap_or(file)
        .replace(['/', '\\'], ".");
    let classes = classname
        .strip_prefix(&module)
        .map(|rest| rest.trim_start_matches('.'))
        .unwrap_or("");
    if classes.is_empty() {
        format!("{file}::{name}")
    } else {
        format!("{file}::{}::{name}", classes.replace('.', "::"))
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

    #[test]
    fn node_ids() {
        assert_eq!(
            node_id("tests/test_a.py", "tests.test_a", "test_x"),
            "tests/test_a.py::test_x"
        );
        assert_eq!(
            node_id("tests/test_a.py", "tests.test_a.TestK", "test_x[1-2]"),
            "tests/test_a.py::TestK::test_x[1-2]"
        );
        assert_eq!(node_id("", "", "tests/test_a.py"), "tests/test_a.py");
    }

    #[test]
    fn junit_counts_and_kinds() {
        let xml = r#"<?xml version="1.0"?><testsuites><testsuite tests="4"><testcase classname="t.m" name="a" file="t/m.py" line="1" time="0"/><testcase classname="t.m" name="b" file="t/m.py" line="4"><failure message="assert 1 == 2">def b():
&gt;   assert 1 == 2
E   assert 1 == 2</failure></testcase><testcase classname="t.m" name="c" file="t/m.py" line="8"><error message="failed on setup with &quot;boom&quot;">trace</error></testcase><testcase classname="t.m" name="d" file="t/m.py" line="9"><skipped message="why"/></testcase></testsuite></testsuites>"#;
        let j = parse_junit(xml).unwrap();
        assert_eq!((j.passed, j.skipped, j.cases.len()), (1, 1, 4));
        let b = &j.cases[1];
        assert_eq!(b.problem.as_ref().unwrap().kind, "failure");
        assert_eq!(
            b.problem.as_ref().unwrap().text,
            "def b():\n>   assert 1 == 2\nE   assert 1 == 2"
        );
        let c = &j.cases[2];
        assert_eq!(
            c.problem.as_ref().unwrap().message,
            "failed on setup with \"boom\""
        );
    }
}
