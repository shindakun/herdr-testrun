# Changelog

## Unreleased

- Build errors are sent to the agent like failures, by `send`, `a`, and watch+send.
- Root detection skips plugin panes (a cwd under `~/.config/herdr/plugins`) and falls back to the workspace's agent pane cwd.
- Auto-run: the idle hook sends `auto-run`; the pane skips it when the worktree is unchanged, and in watch+send mode sends failures back with a round limit and an identical-failures stop.
- Adapters: cargo, jest, vitest, node:test, and pytest parsers, with recorded-output tests and end-to-end fixture runs for all six runners. Failure paths are relative to the project root in monorepos.
- Pane: ratatui screen with the failure list, streaming output, rerun failed, send to agent, auto-run toggle, raw log popup; control socket so `run` and the idle hook can drive an open pane.
- Scaffold: manifest, data model, adapter registry, go adapter, project detection, config file, prompt formatting, state paths, fixture projects, recorded-output parser tests, house tooling and CI.
