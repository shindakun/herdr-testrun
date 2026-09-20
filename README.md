# herdr-testrun

A [Herdr](https://herdr.dev) plugin that runs a project's tests in a split pane, lists the failures, and sends them to the workspace's agent on one key. Rust, one binary. One adapter per test runner: Go, Cargo, Jest, Vitest, `node --test`, and Pytest.

The design is in [docs/PLAN.md](docs/PLAN.md).

## Install

```sh
herdr plugin install shindakun/herdr-testrun
```

Needs `cargo`; the install step builds the binary. Linux and macOS.

For local development, link the checkout instead:

```sh
cargo build --release
herdr plugin link /path/to/herdr-testrun
```

## Use it

Bind the actions in `~/.config/herdr/config.toml`, then `herdr server reload-config`:

```toml
[[keys.command]]
key = "prefix+t"
type = "plugin_action"
command = "shindakun.testrun.run"
description = "run tests"

[[keys.command]]
key = "prefix+shift+t"
type = "plugin_action"
command = "shindakun.testrun.send"
description = "send test failures to agent"
```

`run` finds the project root from the focused pane's cwd (the nearest directory with `.herdr-testrun.toml` or `.git`) and opens the Tests pane beside it. When the focused pane is another plugin's pane, such as a file viewer, it uses the workspace's agent pane instead. The pane runs the tests on start and again on each `run`. `send` formats the last run's build errors and failures as one prompt and gives it to the workspace's agent through `herdr agent prompt`.

The pane:

```text
 Tests  go-basic  ✗ 2 failed  3 passed  1 skipped  0.8s
 ─────────────────────────────────────────────────────────────────────
 ✗ TestFirstFails  internal/parse/parse_test.go:13
 ✗ TestParse/empty_input  internal/parse/parse_test.go:25
 ✓ 3 passed, 1 skipped
 ─────────────────────────────────────────────────────────────────────
 go: TestParse/empty_input
     parse_test.go:25: got "", want "x"
                    r run  f failed  a agent  w watch  enter expand  o log  q quit
```

| Key | Does |
| --- | --- |
| `r` | Run everything |
| `f` | Rerun only the failures from the last run |
| `a` | Send the build errors and failures to the workspace's agent |
| `w` | Cycle watch modes for this project: off, watch, watch+send |
| `enter` | Grow or shrink the detail panel |
| `o` | Open the raw runner output in a popup |
| `j` `k` `g` `G`, arrows, mouse | Move the selection |
| `q` | Quit |

While a run is active the bottom panel streams the runner's output. One run at a time; a second request waits behind it and further requests are dropped.

Watch reruns the tests when the workspace's agent goes idle, and only when the worktree changed since the last run (`git status` and `git diff HEAD`, plus the size and mtime of untracked files). Watch+send also sends the build errors and failures back to the agent after each automatic run, so the agent fixes, the tests rerun, and the loop continues. It stops after three rounds, or when a run produces the same problems as the one before, and the counter resets when a run passes. Both modes are per project and persist across pane restarts.

The binary also works outside Herdr:

```sh
herdr-testrun pane --dir path/to/project       # the pane in the current terminal
herdr-testrun run --dir path/to/project        # ask an open pane to run, else run inline and print
herdr-testrun run --dir path/to/project --json
herdr-testrun send --dir path/to/project --print   # print the prompt instead of sending
herdr-testrun log --dir path/to/project        # page the last raw output
```

`--dir` is the project root as given; without it the detection walk starts at the current directory.

## Runners

| Adapter | Detected by | Runs |
| --- | --- | --- |
| go | `go.mod` | `go test -json ./...` |
| cargo | `Cargo.toml` | `cargo test --no-fail-fast` |
| jest | `jest` in package.json dependencies | `npx jest --json --outputFile=... --testLocationInResults` |
| vitest | `vitest` in package.json dependencies | `npx vitest run --reporter=json --outputFile=...` |
| nodetest | `scripts.test` starts with `node --test` | `node --test --test-reporter=tap` |
| pytest | `pyproject.toml`, `pytest.ini`, or `setup.cfg` with `[tool:pytest]` | `pytest -o junit_family=xunit1 --junitxml=... -q` |

Every matching adapter runs. A root with no markers is checked one level down, so `web/package.json` under a Go root is found. JS runners use `bun x`, `pnpm exec`, or `yarn` when the matching lockfile exists, else `npx`.

Each failure carries the test name, the file and line relative to the project root, and the runner's output for that test (40 lines at most; the rest is in the raw log). Compiler errors, import errors, and timeouts show as a build error for the target.

## Configure

`.herdr-testrun.toml` at the project root replaces detection. The same file can live outside the project at `$(herdr plugin config-dir shindakun.testrun)/<hash>.toml`, where `<hash>` is the first 16 hex characters of the SHA-256 of the root path.

```toml
timeout_secs = 600   # per run; the process group is killed past this

[[target]]
adapter = "cargo"
dir = "."
command = ["cargo", "nextest", "run"]   # optional; the adapter still parses the output

[[target]]
adapter = "jest"
dir = "web"
```

## Development

```sh
make check           # fmt, clippy, unit and parser tests, audit, markdown lint; same as CI
make test-fixtures   # also run the real fixture projects with whatever toolchains are installed
make hooks           # install pre-commit
```

Parser tests read recorded runner output from `tests/output/`. Re-record a fixture with `scripts/record.sh FIXTURE` when a runner changes its format. The fixture test compares each `fixtures/*/expected.json` with a real run and skips fixtures whose toolchain is missing. `make fixture-deps` installs the JS fixtures' `node_modules`; `fixtures/pytest-basic` needs `pytest` on `PATH`.

Adding a runner: one file in `src/adapters/`, one line in `src/adapters/mod.rs`, one fixture project with an `expected.json`, one recording.

## License

MIT.
