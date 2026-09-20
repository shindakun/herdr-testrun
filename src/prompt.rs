//! Formats failures into the one prompt `send` gives the agent.

use crate::model::Failure;

pub const MAX_FAILURES: usize = 25;
pub const MAX_PROMPT_BYTES: usize = 16384;

const HEADER: &str = "These tests fail. Fix the code, not the tests, unless a test is wrong.\n";

/// The prompt for `failures`, capped at [`MAX_FAILURES`] entries and
/// [`MAX_PROMPT_BYTES`]. Past a cap, one line says how many were left out.
pub fn format(failures: &[Failure]) -> String {
    let mut out = String::from(HEADER);
    let mut shown = 0;
    for f in failures.iter().take(MAX_FAILURES) {
        let section = section(f);
        if out.len() + section.len() > MAX_PROMPT_BYTES {
            break;
        }
        out.push_str(&section);
        shown += 1;
    }
    let hidden = failures.len() - shown;
    if hidden > 0 {
        out.push_str(&format!(
            "\n{hidden} more failure{} not shown. Run the tests to see {}.\n",
            if hidden == 1 { "" } else { "s" },
            if hidden == 1 { "it" } else { "them" }
        ));
    }
    out
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
    use crate::model::{AdapterId, RerunKey};
    use std::path::PathBuf;

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
            format(&[f]),
            "These tests fail. Fix the code, not the tests, unless a test is wrong.\n\
             \n## go: TestParse/empty_input (internal/parse/parse_test.go:42)\n\
             \x20   parse_test.go:42: got \"\", want \"x\"\n"
        );
    }

    #[test]
    fn no_location_and_no_output() {
        let mut f = failure(1, "");
        f.file = None;
        f.line = None;
        assert!(format(&[f]).ends_with("\n## go: TestN1\n"));
    }

    #[test]
    fn caps_failure_count() {
        let all: Vec<Failure> = (0..30).map(|i| failure(i, "x")).collect();
        let p = format(&all);
        assert_eq!(p.matches("\n## ").count(), MAX_FAILURES);
        assert!(p.ends_with("5 more failures not shown. Run the tests to see them.\n"));
    }

    #[test]
    fn caps_bytes() {
        let big = "y".repeat(7000);
        let all: Vec<Failure> = (0..5).map(|i| failure(i, &big)).collect();
        let p = format(&all);
        assert!(p.len() <= MAX_PROMPT_BYTES + 80, "{}", p.len());
        assert_eq!(p.matches("\n## ").count(), 2);
        assert!(p.ends_with("3 more failures not shown. Run the tests to see them.\n"));
    }

    #[test]
    fn singular_hidden_line() {
        let all: Vec<Failure> = (0..26).map(|i| failure(i, "x")).collect();
        assert!(format(&all).ends_with("1 more failure not shown. Run the tests to see it.\n"));
    }
}
