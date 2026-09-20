# Changelog

## Unreleased

- A runner that cannot start is that target's build error; the other targets still run. A bare `pyproject.toml` no longer counts as a pytest suite.

## 0.1.0 (2026-09-20)

First release.

- Runs a project's tests in a Herdr split pane and lists build errors and failures with file, line, and the runner's output.
- Adapters for Go, Cargo, Jest, Vitest, `node --test`, and Pytest. Detection by project markers, one level deep for monorepos, or a `.herdr-testrun.toml` naming the targets and an optional command override.
- Rerun all or failed only; stream output while a run is active; open the raw log in a popup.
- `send` and the `a` key give the workspace's agent the build errors and failures as one prompt, capped in count and size.
- Watch reruns when the agent goes idle and the worktree changed. Watch+send also sends the problems back, stopping after three rounds or when the problems repeat.
- `run`, `send`, and `log` work outside Herdr too.
