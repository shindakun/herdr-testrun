# herdr-testrun

A [Herdr](https://herdr.dev) plugin that runs a project's tests in a split pane, lists the failures, and sends them to the workspace's agent on one key. Rust, one binary. One adapter per test runner: Go, Cargo, Jest, Vitest, `node --test`, and Pytest.

Status: scaffold. Detection, the config file, the Go adapter, the `run` and `send` subcommands, the fixture projects, and the parser test harness work. The pane, the other five parsers, and auto-run on agent idle are next. The design and build order are in [docs/PLAN.md](docs/PLAN.md).

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

`run` finds the project root from the focused pane's cwd (the nearest directory with `.herdr-testrun.toml` or `.git`), detects the test runners, runs them, and records the result. `send` formats the last run's failures as one prompt and gives it to the workspace's agent through `herdr agent prompt`.

The binary also works outside Herdr:

```sh
herdr-testrun run --dir path/to/project        # print failures, exit 1 if any
herdr-testrun run --dir path/to/project --json
herdr-testrun send --dir path/to/project --print   # print the prompt instead of sending
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

Only the Go parser is built. The other adapters run their command and report `parser not built yet` until docs/PLAN.md step 5 lands.

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

Parser tests read recorded runner output from `tests/output/`. Re-record a fixture with `scripts/record.sh FIXTURE` when a runner changes its format. The fixture test compares each `fixtures/*/expected.json` with a real run and skips fixtures whose toolchain is missing; `fixtures/jest-basic` and `fixtures/vitest-basic` need `npm install` first, and `fixtures/pytest-basic` needs `pytest` on `PATH`.

Adding a runner: one file in `src/adapters/`, one line in `src/adapters/mod.rs`, one fixture project with an `expected.json`, one recording.

## License

MIT.
