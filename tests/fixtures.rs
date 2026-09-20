//! End-to-end: run each adapter against the real project under `fixtures/`
//! and compare the failures with the fixture's `expected.json`.
//!
//! Opt in with `HERDR_TESTRUN_FIXTURES=1` (`make test-fixtures`); the suite
//! shells out to go, cargo, node, and pytest. A fixture whose toolchain is
//! missing, or whose adapter has no parser yet, is skipped and named.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use herdr_testrun::cli::run_target;
use herdr_testrun::config::Config;
use herdr_testrun::detect;

/// Adapters with a parser. Extend as docs/PLAN.md build order step 5 lands.
const READY: &[&str] = &["go"];

#[derive(Debug, Deserialize)]
struct Expected {
    adapters: Vec<String>,
    failures: Vec<ExpectedFailure>,
    #[serde(default)]
    build_error: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct ExpectedFailure {
    adapter: String,
    name: String,
    file: Option<PathBuf>,
    line: Option<u32>,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// The program an adapter needs on PATH.
fn toolchain(adapter: &str) -> &'static str {
    match adapter {
        "go" => "go",
        "cargo" => "cargo",
        "jest" | "vitest" => "node",
        "nodetest" => "node",
        "pytest" => "pytest",
        other => panic!("no toolchain mapping for {other}"),
    }
}

fn installed(program: &str) -> bool {
    // `go version`, everything else `--version`.
    let flag = if program == "go" {
        "version"
    } else {
        "--version"
    };
    Command::new(program)
        .arg(flag)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// JS fixtures also need their dependencies installed.
fn deps_ready(adapter: &str, dir: &Path) -> bool {
    !matches!(adapter, "jest" | "vitest") || dir.join("node_modules").is_dir()
}

#[test]
fn fixtures_match_expected() {
    if std::env::var_os("HERDR_TESTRUN_FIXTURES").is_none() {
        eprintln!("skipped: set HERDR_TESTRUN_FIXTURES=1 to run the fixture projects");
        return;
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("expected.json").is_file())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no fixtures with expected.json");

    let mut failures = Vec::new();
    let state = std::env::temp_dir().join(format!("herdr-testrun-fixtures-{}", std::process::id()));
    std::fs::create_dir_all(&state).unwrap();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(dir.join("expected.json")).unwrap();
        let expected: Expected =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}/expected.json: {e}"));
        let root = std::fs::canonicalize(&dir).unwrap();
        let config = Config::load(&root, None).unwrap();
        let targets = detect::targets(&root, config.as_ref()).unwrap();
        let detected: BTreeSet<&str> = targets.iter().map(|t| t.adapter.id()).collect();
        let wanted: BTreeSet<&str> = expected.adapters.iter().map(String::as_str).collect();
        if detected != wanted {
            failures.push(format!(
                "{name}: detected {detected:?}, expected {wanted:?}"
            ));
            continue;
        }

        let mut got: Vec<ExpectedFailure> = Vec::new();
        let mut build_error = false;
        let mut ran: Vec<&str> = Vec::new();
        for target in &targets {
            let id = target.adapter.id();
            if !READY.contains(&id) {
                eprintln!("{name}: skipped {id} (parser not built yet)");
                continue;
            }
            if !installed(toolchain(id)) || !deps_ready(id, &target.dir) {
                eprintln!("{name}: skipped {id} (toolchain or dependencies missing)");
                continue;
            }
            let (result, _log) = run_target(target, &state, Duration::from_secs(300)).unwrap();
            build_error |= result.build_error.is_some();
            got.extend(result.failures.iter().map(|f| ExpectedFailure {
                adapter: f.adapter.to_string(),
                name: f.name.clone(),
                file: f.file.clone(),
                line: f.line,
            }));
            ran.push(id);
        }
        if ran.is_empty() {
            continue;
        }
        let mut want: Vec<&ExpectedFailure> = expected
            .failures
            .iter()
            .filter(|f| ran.contains(&f.adapter.as_str()))
            .collect();
        want.sort();
        got.sort();
        let got_refs: Vec<&ExpectedFailure> = got.iter().collect();
        if got_refs != want {
            failures.push(format!(
                "{name}: failures\n  got:  {got:?}\n  want: {want:?}"
            ));
        }
        if expected.build_error != build_error {
            failures.push(format!(
                "{name}: build_error {build_error}, expected {}",
                expected.build_error
            ));
        }
        eprintln!("{name}: ok ({})", ran.join(", "));
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
