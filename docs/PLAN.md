# herdr-testrun

A herdr plugin that runs a project's tests in a split pane, lists failures,
and sends them to the workspace's agent on one key. One Rust binary. One
adapter per test runner. Real fixture projects to test the adapters against.

## Goals

- Run tests for Go, Rust, Jest, Vitest, node:test, and Pytest projects.
- Show failures as a list: name, file, line, trimmed output.
- Rerun all, rerun failed only, expand one failure.
- Send failures to the focused workspace's agent as one prompt.
- Optional rerun when the agent goes idle, with loop guards.
- Adding a runner means one adapter file and one fixture project.

## Non-goals

- Watching files. Herdr events drive reruns, not inotify.
- Coverage. Separate plugin.
- Editing tests. The agent does that.
- A shared JUnit conversion layer. Each adapter parses its runner natively.

## Repo layout

```text
herdr-testrun/
  Cargo.toml              single crate, lib plus bin; excludes fixtures/
  herdr-plugin.toml       manifest
  README.md
  docs/PLAN.md
  src/
    main.rs               argv dispatch: pane | run | send | on-agent-idle
    lib.rs                module list
    cli.rs                the one-shot subcommands
    tui.rs                ratatui pane
    runner.rs             spawn, capture, timeout, process-group kill
    model.rs              Failure, RunResult, RerunKey, Adapter trait
    detect.rs             walk up from cwd, match markers, one level down
    config.rs             .herdr-testrun.toml
    herdr.rs              plugin env, context JSON, event JSON, agent list
    prompt.rs             format failures into one agent prompt, size caps
    state.rs              per-root state dir, last.json, settings.json
    testutil.rs           tempdir and argv helpers, cfg(test) only
    adapters/
      mod.rs              registry: [go, cargo, jest, vitest, nodetest, pytest]
      go.rs
      cargo.rs
      jest.rs
      vitest.rs
      nodetest.rs
      pytest.rs
  fixtures/
    go-basic/             two packages; one failing test, one failing subtest, one skip
    go-build-fail/        one package that does not compile
    cargo-basic/          unit test assertion failure, integration test panic
    jest-basic/           one failing assertion
    vitest-basic/         one failing assertion
    nodetest-basic/       "test": "node --test", one failing assertion
    pytest-basic/         one failing test, one error in fixture setup
    mixed-go-jest/        go.mod at root, web/package.json with jest, config names both
    override-config/      cargo project whose config replaces the command
  tests/
    adapters.rs           parse recorded output in tests/output/
    fixtures.rs           run real fixtures end to end, opt in with HERDR_TESTRUN_FIXTURES=1
    fixtures/             recorded herdr output: agent_list.json, agent_status_event.json
    output/
      go-basic.json       recorded `go test -json`, plus .code with the exit status
      cargo-basic.txt     recorded `cargo test`, plus .stderr
      ...
  scripts/
    record.sh             run a fixture's runner, write tests/output/
    release.sh            bump, tag, push, GitHub release
```

Fixture projects live in the repo but are not Cargo workspace members. The
root `Cargo.toml` excludes `fixtures/`. Each fixture is a complete, tiny
project that a human can `cd` into and run by hand, with an `expected.json`
listing the failures a run must produce.

## Data model

```rust
pub struct AdapterId(pub &'static str);  // deserializes by registry lookup

pub struct Failure {
    pub adapter: AdapterId,      // "go", "cargo", ...
    pub name: String,            // "TestFoo/sub", "tests::foo", "adds two numbers"
    pub file: Option<PathBuf>,   // relative to project root
    pub line: Option<u32>,
    pub output: String,          // trimmed to MAX_OUTPUT_LINES
    pub rerun: RerunKey,         // what the adapter needs to run just this one
}

pub struct RunResult {
    pub adapter: AdapterId,
    pub root: PathBuf,
    pub started: SystemTime,
    pub duration: Duration,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub failures: Vec<Failure>,
    pub exit_code: i32,
    pub raw_log: PathBuf,        // full output on disk for the expand view
    pub build_error: Option<String>,
}

pub trait Adapter {
    fn id(&self) -> &'static str;
    fn detect(&self, root: &Path) -> bool;
    fn command(&self, root: &Path, state: &Path, only: Option<&[RerunKey]>) -> Command;
    fn parse(&self, root: &Path, state: &Path, stdout: &str, stderr: &str, code: i32) -> RunResult;
}
```

`state` is the project's state directory. Jest, Vitest, and Pytest write
their result file there and read it back in `parse`; the other adapters
ignore it.

`RerunKey` is an enum: `GoTest { pkg, name }`, `CargoTest { name }`,
`JestTest { file, name }`, `VitestTest { file, name }`,
`NodeTest { file, name }`, `PytestNode { id }`. Only the adapter that made a
key reads it.

## Adapters

| Adapter | Detect | Command | Parse |
|---|---|---|---|
| go | `go.mod` | `go test -json ./...` | stream of `{Action,Package,Test,Output}`. `Action=fail` with `Test` set is a failure; its `output` events are the message, minus the `=== RUN` and `--- FAIL` frame lines. A parent whose subtest failed is not listed. Go 1.24+ reports build errors as `build-output` events; older versions print them to stderr and emit an `output` line containing `[build failed]`. Both set `build_error`. |
| cargo | `Cargo.toml` | `cargo test --no-fail-fast` | stable libtest text. `test NAME ... FAILED` lists failures. `---- NAME stdout ----` blocks carry output until the next `----` or `failures:` line. `panicked at FILE:LINE:COL` gives file and line. Compiler errors before the `running N tests` line are a build error. Exit code 101 on failure. |
| jest | `package.json` deps or devDeps has `jest` | `npx jest --json --outputFile=STATE/jest.json --testLocationInResults` | `testResults[].assertionResults[]` with `status == "failed"`. `fullName`, `failureMessages[]`. `location.line` is the test's definition line. File is `testResults[].name`, absolute; make it relative to root. |
| vitest | deps or devDeps has `vitest` | `npx vitest run --reporter=json --outputFile=STATE/vitest.json` | same shape as jest but no `location`. Line comes from the first in-project frame of `failureMessages[0]` (`at ROOT/file:LINE:COL`). |
| nodetest | `scripts.test` starts with `node --test` | `node --test --test-reporter=tap` | TAP: `not ok N - NAME` then an indented YAML block with `location`, `failureType`, `error`, `stack`. File and line from the first `stack` frame in the project (the assertion), not `location` (the test definition). |
| pytest | `pyproject.toml`, `pytest.ini`, `setup.cfg` with `[tool:pytest]` | `pytest -o junit_family=xunit1 --junitxml=STATE/junit.xml -q` | `<testcase classname file line name>` with a `<failure>` or `<error>` child. Message is the child's text. The default `xunit2` family omits `file` and `line`, so the command forces `xunit1`. Its `line` is 0-based; add 1. Node id is `FILE::NAME`. |

Rerun failed only:

| Adapter | Command |
|---|---|
| go | `go test -json PKG... -run '^(TOP1\|TOP2)$'`. `-run` cannot pick one subtest per parent across parents in one regex, so a failed subtest reruns its whole top-level test; accept it |
| cargo | `cargo test --no-fail-fast -- NAME1 NAME2` (substring match, so exact names may run extra tests; accept it) |
| jest | `npx jest --json --outputFile=... --testLocationInResults FILE... -t '^(NAME1\|NAME2)$'` |
| vitest | `npx vitest run --reporter=json --outputFile=... FILE... -t '^(NAME1\|NAME2)$'` |
| nodetest | `node --test --test-reporter=tap --test-name-pattern='^NAME$'... FILE...` |
| pytest | `pytest -o junit_family=xunit1 --junitxml=... NODEID...` |

Package manager for JS: `bun x` if `bun.lockb` or `bun.lock` exists,
`pnpm exec` if `pnpm-lock.yaml`, `yarn` if `yarn.lock`, else `npx`. Detect
once per run.

Rust JSON test output is nightly only. Do not use it. The libtest text
format has not changed in years and is the stable interface.

## Detection

1. Start from `--dir` when given; it is the root as given, no walk.
   Otherwise start from `focused_pane_cwd` in `HERDR_PLUGIN_CONTEXT_JSON`,
   falling back to `workspace_cwd`, then the process cwd.
2. Walk up from there. Stop at the first directory containing
   `.herdr-testrun.toml` or `.git`. That is the project root. With neither,
   the start directory is the root.
3. If a config file exists, use it and skip marker detection.
4. Otherwise run every adapter's `detect` on the root. All matches run.
5. For monorepos, also check one level of subdirectories for markers when
   the root has none, so `web/package.json` under a Go root is found.
   Hidden directories and `node_modules`, `target`, `vendor`, `dist`,
   `build` are skipped. A root that has markers of its own stops the scan;
   a monorepo with markers at both levels names both in the config file.

Config file, `.herdr-testrun.toml` at project root or in
`HERDR_PLUGIN_CONFIG_DIR/<sha of root path>.toml` (first 16 hex characters
of SHA-256 over the path string). The project file wins when both exist.

```toml
timeout_secs = 600                       # optional, per run

[[target]]
adapter = "cargo"
dir = "."
command = ["cargo", "nextest", "run"]   # optional override, parse still uses the adapter

[[target]]
adapter = "jest"
dir = "web"
```

Unknown keys and unknown adapter ids are errors.

## Herdr wiring

`herdr-plugin.toml`:

```toml
id = "shindakun.testrun"
name = "Test Run"
version = "0.1.0"
min_herdr_version = "0.9.0"
description = "Run tests in a pane, send failures to the agent"
platforms = ["linux", "macos"]

[[build]]
command = ["cargo", "build", "--release"]

[[panes]]
id = "tests"
title = "Tests"
placement = "split"
command = ["./target/release/herdr-testrun", "pane"]

[[actions]]
id = "run"
title = "Run tests"
contexts = ["workspace", "pane"]
command = ["./target/release/herdr-testrun", "run"]

[[actions]]
id = "send"
title = "Send failures to agent"
contexts = ["workspace", "pane"]
command = ["./target/release/herdr-testrun", "send"]

[[events]]
on = "pane.agent_status_changed"
command = ["./target/release/herdr-testrun", "on-agent-idle"]
```

Subcommands:

- `pane`: the TUI. Long-lived. Owns the run queue.
- `run [--dir PATH] [--json]`: with a pane open for this root, ask it to run.
  With no pane, or outside Herdr, run inline and print the failures; exit 1
  when any fail. Records `last.json` and `raw.log` either way.
- `send [--dir PATH] [--print]`: format the last result for this root and
  call `herdr agent prompt`. `--print` writes the prompt to stdout instead.
- `on-agent-idle`: read `HERDR_PLUGIN_EVENT_JSON`. If the status went to
  `idle` or `done` and auto-run is on for this root, apply the guards below,
  then behave like `run`.

The pane and the one-shot commands talk over a Unix socket in the root's
state directory. Messages: `run`, `run-failed`, `status`. The one-shot
commands exit right after sending.

What herdr 0.9.1 gives each process, from its source
(`src/app/api/plugins/runtime.rs`, `panes.rs`, `context.rs`) and a live
session:

- Every plugin process gets `HERDR_PLUGIN_CONTEXT_JSON`. Pane commands get it
  too (`plugin_pane_launch_env` sets it unconditionally), holding the active
  workspace and its focused pane at the moment the pane opened:
  `workspace_id`, `workspace_cwd`, `focused_pane_id`, `focused_pane_cwd`,
  `focused_pane_agent`, `focused_pane_status`. It is a snapshot; a pane
  that outlives focus changes gets fresh cwds from the `run` messages the
  actions send it, each carrying that action's context.
- `pane.agent_status_changed` carries the workspace id. The hook's
  `HERDR_PLUGIN_EVENT_JSON` is the event envelope:
  `{"event":"pane_agent_status_changed","data":{"type":"pane_agent_status_changed","pane_id":"w3:p1","workspace_id":"w3","agent_status":"idle","agent":"claude"}}`.
  The hook's context JSON is built from that pane, so `focused_pane_cwd` is
  the agent pane's cwd, and `HERDR_WORKSPACE_ID` and `HERDR_PANE_ID` are set.
  No `herdr pane get` call is needed. `agent_status` is one of `idle`,
  `working`, `blocked`, `done`, `unknown`; `idle` and `done` both mean ready.
- `herdr agent prompt TARGET TEXT` writes the text and Enter and returns once
  the writes are acknowledged. It does not wait for the agent's turn. It
  fails with `agent_blocked` when the agent is on an approval prompt, and
  with `--wait` it blocks until the agent settles. `send` calls it without
  `--wait` and reports the error; no `agent wait` first.
- `herdr agent list` prints JSON with no flag; there is no `--json`. Rows
  carry `pane_id`, `workspace_id`, `agent_status`, `focused`, `agent`, `cwd`.

Agent selection for `send`: `herdr agent list`, filter by the context's
`workspace_id`. One agent: use it. Several: prefer the context's
`focused_pane_id`, then the row marked `focused`, else the first. None: print
an error and exit 1. The target passed to `agent prompt` is the pane id.

## The prompt

```text
These tests fail. Fix the code, not the tests, unless a test is wrong.

## go: TestParse/empty_input (internal/parse/parse_test.go:42)
    parse_test.go:42: got "", want "x"

## cargo: tests::roundtrip (src/lib.rs:88)
    thread 'tests::roundtrip' panicked at src/lib.rs:88:9:
    assertion `left == right` failed
      left: 1
     right: 2
```

Caps: `MAX_OUTPUT_LINES = 40` per failure, `MAX_FAILURES = 25` in one
prompt, `MAX_PROMPT_BYTES = 16384`. Past the caps, append one line:
`N more failures not shown. Run the tests to see them.`

## Auto-run guards

Rerun on agent idle is the point of the plugin and the way it goes wrong.
Rules, all enforced in the pane process:

- Change gate: hash `git status --porcelain` plus `git diff` for the
  worktree. Skip the run if the hash matches the last run.
- One run at a time per worktree. If a run is active, queue at most one
  more. Extra requests are dropped.
- Auto-send is off by default. When on: stop after `max_rounds` (default 3)
  sends per session, and stop when the failure set is identical to the
  previous run. Reset the counter when a run passes.
- Timeout per run, default 600s, `timeout_secs` in the config file. On
  timeout kill the process group and set `build_error` to `timed out`.

## TUI

ratatui, crossterm. One screen.

```text
 Tests  go-basic  ✗ 2 failed  12 passed  0.8s        [r]un [f]ailed [a]gent [w]atch [q]uit
 ─────────────────────────────────────────────────────────────────────────
 ✗ TestParse/empty_input        internal/parse/parse_test.go:42
 ✗ TestServer_Timeout           internal/server/server_test.go:118
 ✓ 12 passed (collapsed)
 ─────────────────────────────────────────────────────────────────────────
 parse_test.go:42: got "", want "x"
```

Keys: `r` run all, `f` rerun failed, `a` send to agent, `w` toggle auto-run
for this worktree, `enter` expand or collapse a failure, `o` open the raw
log in a popup via `herdr plugin pane open` with `placement=popup`, `j/k`
move, `q` quit. Mouse click selects a row.

While a run is active, stream the raw output in the bottom panel and show
a spinner in the header. Replace it with the parsed list when the run ends.

## State

`HERDR_PLUGIN_STATE_DIR/<sha of root path>/`:

```text
last.json       Vec<RunResult>, one per target, minus raw output
raw.log         full stdout+stderr of the last run, all targets
settings.json   { auto_run: bool, auto_send: bool, max_rounds: u32 }
jest.json, vitest.json, junit.xml   adapter result files
```

Files are written to a sibling temp name and renamed. The pane loads
`last.json` on start so a restart shows the previous run. Outside Herdr the
state dir is `$TMPDIR/herdr-testrun`.

## Testing

Two layers.

Parser tests (`tests/adapters.rs`): recorded runner output in
`tests/output/`, one file per fixture per runner version, plus `.code` with
the exit status and `.stderr` when it was non-empty. Each test loads the
file, calls `parse`, and asserts the exact `Failure` list. These run in CI
with no toolchains beyond Rust. `scripts/record.sh FIXTURE` runs the real
runner and writes the files; it rewrites the fixture's absolute path to
`/FIXTURE_ROOT` and blanks pytest's hostname so recordings match on every
machine. Re-record when a runner changes its format.

Fixture tests (`tests/fixtures.rs`): run the real adapter against each
`fixtures/*` project end to end and compare with its `expected.json`
(adapter, name, file, line per failure, plus whether a build error is
expected). Opt in with `HERDR_TESTRUN_FIXTURES=1`. A fixture is skipped, and
named, when its toolchain is missing, when a JS fixture has no
`node_modules`, or when the adapter's parser is not built. Toolchains on the
dev machine: cargo, go, node, pnpm, yarn. Missing: bun, pytest; the pytest
fixture runs through a venv with `pytest` on `PATH`.

Unit tests in each module for: detection walk, config parsing, prompt
formatting and caps, rerun command construction, state round trips, herdr
JSON parsing, runner timeout. The change-gate hash lands with the pane.

CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, `cargo audit`, `markdownlint-cli2`. Fixture tests run on a job
with Go, Node, and Python installed.

## Build order

1. `model.rs`, `adapters/go.rs`, parser test with recorded output. Done.
2. `runner.rs`, `detect.rs`, a `run` subcommand that prints failures to
   stdout. Works with no herdr present. Done against `fixtures/go-basic`.
3. `tui.rs`, the socket, and `run` delegating to an open pane. Test against
   go-basic by hand.
4. `send` against a live herdr session. The code is in; it has not been run
   under Herdr.
5. cargo, jest, vitest, nodetest, pytest parsers. Fixtures and recordings
   exist; add each id to `READY` in `tests/fixtures.rs` as its parser lands.
6. `on-agent-idle` guards in the pane. The hook parses the event and reads
   settings; the guarded rerun is the pane's.
7. Manifest, install from GitHub, tag `herdr-plugin`.
