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
