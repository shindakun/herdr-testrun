//! Formats a run's problems, build errors and failures, into the one prompt
//! `send` gives the agent.

use std::collections::BTreeSet;

use crate::model::{trim_output, AdapterId, Failure, RunResult};

pub const MAX_FAILURES: usize = 25;
pub const MAX_PROMPT_BYTES: usize = 16384;

/// A target that never got to its tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    pub adapter: AdapterId,
    pub text: String,
}

/// Everything a run produced that the agent should fix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Problems {
    pub build_errors: Vec<BuildError>,
    pub failures: Vec<Failure>,
}

impl Problems {
    pub fn from_results(results: &[RunResult]) -> Self {
        Self {
            build_errors: results
                .iter()
                .filter_map(|r| {
                    r.build_error.as_ref().map(|text| BuildError {
                        adapter: r.adapter,
                        text: text.clone(),
                    })
                })
                .collect(),
            failures: results
                .iter()
                .flat_map(|r| r.failures.iter().cloned())
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.build_errors.is_empty() && self.failures.is_empty()
    }

    pub fn count(&self) -> usize {
        self.build_errors.len() + self.failures.len()
    }

    /// One key per problem, for the "same as last run" stop. A build error
    /// keys on its first line so a moved caret does not read as progress.
    pub fn key_set(&self) -> BTreeSet<String> {
        self.build_errors
            .iter()
            .map(|b| {
                format!(
                    "{}:build:{}",
                    b.adapter,
                    b.text.lines().next().unwrap_or("")
                )
            })
            .chain(
                self.failures
                    .iter()
                    .map(|f| format!("{}:{}", f.adapter, f.name)),
            )
            .collect()
    }

    /// `2 failures`, `1 build error and 3 failures`.
    pub fn describe(&self) -> String {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        match (self.build_errors.len(), self.failures.len()) {
            (0, f) => plural(f, "failure", "failures"),
            (b, 0) => plural(b, "build error", "build errors"),
            (b, f) => format!(
                "{} and {}",
                plural(b, "build error", "build errors"),
                plural(f, "failure", "failures")
            ),
        }
    }
}

/// The prompt for `problems`: build errors first, then up to
/// [`MAX_FAILURES`] failures, within [`MAX_PROMPT_BYTES`]. Past a cap, one
/// line says how many were left out.
pub fn format(problems: &Problems) -> String {
    let mut out = String::from(if problems.failures.is_empty() {
        "The build fails. Fix the code.\n"
    } else {
        "These tests fail. Fix the code, not the tests, unless a test is wrong.\n"
    });
    let mut shown = 0;
    let sections = problems
        .build_errors
        .iter()
        .map(build_section)
        .chain(problems.failures.iter().take(MAX_FAILURES).map(section));
    for text in sections {
        if out.len() + text.len() > MAX_PROMPT_BYTES {
            break;
        }
        out.push_str(&text);
        shown += 1;
    }
    let hidden = problems.count() - shown;
    if hidden > 0 {
        out.push_str(&format!(
            "\n{hidden} more {} not shown. Run the tests to see {}.\n",
            if hidden == 1 { "problem" } else { "problems" },
            if hidden == 1 { "it" } else { "them" }
        ));
    }
    out
}

fn build_section(b: &BuildError) -> String {
    format!(
        "\n## {}: build failed\n{}\n",
        b.adapter,
        trim_output(&b.text)
    )
}

fn section(f: &Failure) -> String {
    let mut s = format!("\n## {}: {}", f.adapter, f.name);
    match (&f.file, f.line) {
        (Some(file), Some(line)) => s.push_str(&format!(" ({}:{line})", file.display())),
        (Some(file), None) => s.push_str(&format!(" ({})", file.display())),
        _ => {}
    }
    s.push('\n');
    if !f.output.is_empty() {
        s.push_str(&f.output);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RerunKey;
    use std::path::{Path, PathBuf};

    fn failure(n: usize, output: &str) -> Failure {
        Failure {
            adapter: AdapterId("go"),
            name: format!("TestN{n}"),
            file: Some(PathBuf::from("a_test.go")),
            line: Some(n as u32),
            output: output.to_string(),
            rerun: RerunKey::GoTest {
                pkg: "m".into(),
                name: format!("TestN{n}"),
            },
        }
    }

    fn only(failures: Vec<Failure>) -> Problems {
        Problems {
            build_errors: Vec::new(),
            failures,
        }
    }

    #[test]
    fn one_failure() {
        let f = Failure {
            adapter: AdapterId("go"),
            name: "TestParse/empty_input".into(),
            file: Some(PathBuf::from("internal/parse/parse_test.go")),
            line: Some(42),
            output: "    parse_test.go:42: got \"\", want \"x\"".into(),
            rerun: RerunKey::GoTest {
                pkg: "m".into(),
                name: "TestParse/empty_input".into(),
            },
        };
        assert_eq!(
            format(&only(vec![f])),
            "These tests fail. Fix the code, not the tests, unless a test is wrong.\n\
             \n## go: TestParse/empty_input (internal/parse/parse_test.go:42)\n\
             \x20   parse_test.go:42: got \"\", want \"x\"\n"
        );
    }

    #[test]
    fn build_error_alone() {
        let mut r = RunResult::new("cargo", Path::new("/p"), 101);
        r.build_error = Some("error: missing `fn`\n --> src/main.rs:5:2".into());
        let p = Problems::from_results(&[r]);
        assert_eq!(p.describe(), "1 build error");
        assert_eq!(
            format(&p),
            "The build fails. Fix the code.\n\n## cargo: build failed\nerror: missing `fn`\n --> src/main.rs:5:2\n"
        );
        assert_eq!(p.key_set().len(), 1);
        assert!(p
            .key_set()
            .iter()
            .next()
            .unwrap()
            .starts_with("cargo:build:error: missing"));
    }

    #[test]
    fn build_error_comes_before_failures() {
        let mut r = RunResult::new("go", Path::new("/p"), 1);
        r.build_error = Some("# m/p\nbad".into());
        r.failures.push(failure(1, "x"));
        let p = Problems::from_results(&[r]);
        assert_eq!(p.describe(), "1 build error and 1 failure");
        let text = format(&p);
        assert!(text.starts_with("These tests fail."));
        assert!(text.find("## go: build failed").unwrap() < text.find("## go: TestN1").unwrap());
    }

    #[test]
    fn no_location_and_no_output() {
        let mut f = failure(1, "");
        f.file = None;
        f.line = None;
        assert!(format(&only(vec![f])).ends_with("\n## go: TestN1\n"));
    }

    #[test]
    fn caps_failure_count() {
        let all: Vec<Failure> = (0..30).map(|i| failure(i, "x")).collect();
        let p = format(&only(all));
        assert_eq!(p.matches("\n## ").count(), MAX_FAILURES);
        assert!(p.ends_with("5 more problems not shown. Run the tests to see them.\n"));
    }

    #[test]
    fn caps_bytes() {
        let big = "y".repeat(7000);
        let all: Vec<Failure> = (0..5).map(|i| failure(i, &big)).collect();
        let p = format(&only(all));
        assert!(p.len() <= MAX_PROMPT_BYTES + 80, "{}", p.len());
        assert_eq!(p.matches("\n## ").count(), 2);
        assert!(p.ends_with("3 more problems not shown. Run the tests to see them.\n"));
    }

    #[test]
    fn singular_hidden_line() {
        let all: Vec<Failure> = (0..26).map(|i| failure(i, "x")).collect();
        assert!(
            format(&only(all)).ends_with("1 more problem not shown. Run the tests to see it.\n")
        );
    }

    #[test]
    fn describe_counts() {
        assert_eq!(only(vec![failure(1, "")]).describe(), "1 failure");
        assert_eq!(
            only((0..3).map(|i| failure(i, "")).collect()).describe(),
            "3 failures"
        );
    }
}
