//! Parser tests over recorded runner output in `tests/output/`. These need
//! no toolchain beyond Rust. Re-record with `scripts/record.sh FIXTURE`.

use std::path::{Path, PathBuf};

use herdr_testrun::adapters;
use herdr_testrun::model::{RerunKey, RunResult};

fn recorded(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/output")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn code(fixture: &str) -> i32 {
    recorded(&format!("{fixture}.code")).trim().parse().unwrap()
}

fn stderr(fixture: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/output")
        .join(format!("{fixture}.stderr"));
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The fixture directory is the root the recording ran in, so `go.mod` and
/// friends resolve the same way they did then.
fn fixture_root(fixture: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(fixture)
}

fn parse(adapter: &str, fixture: &str, stdout_file: &str) -> RunResult {
    let a = adapters::by_id(adapter).unwrap();
    a.parse(
        &fixture_root(fixture),
        Path::new("/nonexistent-state"),
        &recorded(stdout_file),
        &stderr(fixture),
        code(fixture),
    )
}

#[test]
fn go_basic() {
    let r = parse("go", "go-basic", "go-basic.json");
    assert_eq!(r.exit_code, 1);
    assert_eq!(r.build_error, None);
    assert_eq!((r.passed, r.failed, r.skipped), (3, 2, 1));
    assert_eq!(r.failures.len(), 2);

    let f = &r.failures[0];
    assert_eq!(f.name, "TestFirstFails");
    assert_eq!(
        f.file.as_deref(),
        Some(Path::new("internal/parse/parse_test.go"))
    );
    assert_eq!(f.line, Some(13));
    assert_eq!(f.output, "    parse_test.go:13: got \"a\", want \"b\"");
    assert_eq!(
        f.rerun,
        RerunKey::GoTest {
            pkg: "example.com/gobasic/internal/parse".into(),
            name: "TestFirstFails".into(),
        }
    );

    let f = &r.failures[1];
    assert_eq!(f.name, "TestParse/empty_input");
    assert_eq!(f.line, Some(25));
    assert_eq!(f.output, "    parse_test.go:25: got \"\", want \"x\"");
    assert!(
        !r.failures.iter().any(|f| f.name == "TestParse"),
        "the parent of a failed subtest is not listed"
    );
    assert!(!r.ok());
}

#[test]
fn go_build_fail() {
    let r = parse("go", "go-build-fail", "go-build-fail.json");
    assert_eq!(r.exit_code, 1);
    assert!(r.failures.is_empty());
    let err = r.build_error.as_deref().expect("build error");
    assert!(err.contains("broken.go:5:"), "{err}");
    assert!(err.contains("too many return values"), "{err}");
    assert!(!r.ok());
}

#[test]
fn go_build_fail_on_older_toolchains_reads_stderr() {
    // Before Go 1.24 build errors went to stderr and the JSON only had the
    // `[build failed]` frame line.
    let stdout = r#"{"Action":"start","Package":"m/p"}
{"Action":"output","Package":"m/p","Output":"FAIL\tm/p [build failed]\n"}
{"Action":"fail","Package":"m/p","Elapsed":0}
"#;
    let stderr = "# m/p\n./p.go:5:17: too many return values\n";
    let r =
        adapters::by_id("go")
            .unwrap()
            .parse(Path::new("/p"), Path::new("/s"), stdout, stderr, 1);
    assert_eq!(r.build_error.as_deref(), Some(stderr.trim()));
}

#[test]
fn go_ignores_lines_that_are_not_json() {
    let stdout = "go: downloading example.com/dep v1.0.0\n{\"Action\":\"run\",\"Package\":\"m\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"m\",\"Test\":\"TestA\"}\n";
    let r = adapters::by_id("go")
        .unwrap()
        .parse(Path::new("/p"), Path::new("/s"), stdout, "", 0);
    assert_eq!(r.passed, 1);
    assert!(r.ok());
}

/// A state dir holding the recorded report under the name the adapter reads.
fn state_with(recorded: &str, as_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-testrun-adapters-{}-{recorded}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(as_name), recorded_bytes(recorded)).unwrap();
    dir
}

fn recorded_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/output")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Recordings replace the fixture's absolute path with this.
const FIXTURE_ROOT: &str = "/FIXTURE_ROOT";

#[test]
fn cargo_basic() {
    let r = parse("cargo", "cargo-basic", "cargo-basic.txt");
    assert_eq!(r.exit_code, 101);
    assert_eq!(r.build_error, None);
    assert_eq!((r.passed, r.failed, r.skipped), (2, 2, 0));
    let names: Vec<&str> = r.failures.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["tests::roundtrip_keeps_input", "integration_panics"]
    );

    let f = &r.failures[0];
    assert_eq!(f.file.as_deref(), Some(Path::new("src/lib.rs")));
    assert_eq!(f.line, Some(24));
    assert!(
        f.output
            .starts_with("thread 'tests::roundtrip_keeps_input'"),
        "{}",
        f.output
    );
    assert!(f.output.contains("left: \"bc\""));
    assert!(!f.output.contains("RUST_BACKTRACE"));
    assert_eq!(
        f.rerun,
        RerunKey::CargoTest {
            name: "tests::roundtrip_keeps_input".into()
        }
    );

    let f = &r.failures[1];
    assert_eq!(f.file.as_deref(), Some(Path::new("tests/integration.rs")));
    assert_eq!(f.line, Some(4));
    assert!(f.output.contains("index out of bounds"));
}

#[test]
fn jest_basic() {
    let state = state_with("jest-basic.json", "jest.json");
    let r = adapters::by_id("jest").unwrap().parse(
        Path::new(FIXTURE_ROOT),
        &state,
        "",
        &stderr("jest-basic"),
        code("jest-basic"),
    );
    assert_eq!(r.build_error, None);
    assert_eq!((r.passed, r.failed, r.skipped), (1, 1, 0));
    let f = &r.failures[0];
    assert_eq!(f.name, "doubles a number");
    assert_eq!(f.file.as_deref(), Some(Path::new("math.test.js")));
    assert_eq!(
        f.line,
        Some(7),
        "location.line is the test's definition line"
    );
    assert!(
        f.output
            .starts_with("Error: expect(received).toBe(expected)"),
        "{}",
        f.output
    );
    assert!(f
        .output
        .contains("at Object.toBe (/FIXTURE_ROOT/math.test.js:8:21)"));
    assert!(!f.output.contains("node_modules"), "{}", f.output);
    assert_eq!(
        f.rerun,
        RerunKey::JestTest {
            file: "math.test.js".into(),
            name: "doubles a number".into()
        }
    );
}

#[test]
fn vitest_basic() {
    let state = state_with("vitest-basic.json", "vitest.json");
    let r = adapters::by_id("vitest").unwrap().parse(
        Path::new(FIXTURE_ROOT),
        &state,
        "",
        "",
        code("vitest-basic"),
    );
    assert_eq!(r.build_error, None);
    assert_eq!((r.passed, r.failed, r.skipped), (1, 1, 0));
    let f = &r.failures[0];
    assert_eq!(f.name, "doubles a number");
    assert_eq!(f.file.as_deref(), Some(Path::new("math.test.js")));
    assert_eq!(
        f.line,
        Some(9),
        "line from the first in-project stack frame"
    );
    assert!(
        f.output.starts_with("AssertionError: expected 5 to be 4"),
        "{}",
        f.output
    );
    assert!(!f.output.contains("node_modules"), "{}", f.output);
    assert_eq!(
        f.rerun,
        RerunKey::VitestTest {
            file: "math.test.js".into(),
            name: "doubles a number".into()
        }
    );
}

#[test]
fn nodetest_basic() {
    let r = adapters::by_id("nodetest").unwrap().parse(
        Path::new(FIXTURE_ROOT),
        Path::new("/s"),
        &recorded("nodetest-basic.tap"),
        "",
        code("nodetest-basic"),
    );
    assert_eq!(r.build_error, None);
    assert_eq!((r.passed, r.failed, r.skipped), (1, 1, 0));
    let f = &r.failures[0];
    assert_eq!(f.name, "doubles a number");
    assert_eq!(f.file.as_deref(), Some(Path::new("test/math.test.js")));
    assert_eq!(
        f.line,
        Some(10),
        "the assertion's frame, not the test's location"
    );
    assert_eq!(f.output, "Expected values to be strictly equal:\n\n5 !== 4");
    assert_eq!(
        f.rerun,
        RerunKey::NodeTest {
            file: "test/math.test.js".into(),
            name: "doubles a number".into()
        }
    );
}

#[test]
fn pytest_basic() {
    let state = state_with("pytest-basic.xml", "junit.xml");
    let r = adapters::by_id("pytest").unwrap().parse(
        &fixture_root("pytest-basic"),
        &state,
        &recorded("pytest-basic.stdout"),
        "",
        code("pytest-basic"),
    );
    assert_eq!(r.build_error, None);
    assert_eq!((r.passed, r.failed, r.skipped), (1, 2, 0));
    let f = &r.failures[0];
    assert_eq!(f.name, "tests/test_mathx.py::test_double");
    assert_eq!(f.file.as_deref(), Some(Path::new("tests/test_mathx.py")));
    assert_eq!(f.line, Some(10), "junit line is 0-based");
    assert!(
        f.output
            .starts_with("def test_double():\n>       assert double(2) == 4"),
        "{}",
        f.output
    );
    assert_eq!(
        f.rerun,
        RerunKey::PytestNode {
            id: "tests/test_mathx.py::test_double".into()
        }
    );
    let f = &r.failures[1];
    assert_eq!(f.name, "tests/test_mathx.py::test_uses_broken_fixture");
    assert_eq!(f.line, Some(19));
    assert!(
        f.output
            .starts_with("failed on setup with \"RuntimeError: fixture setup fails on purpose\"\n"),
        "{}",
        f.output
    );
}
