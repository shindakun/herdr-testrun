# Changelog

## Unreleased

- Adapters: cargo, jest, vitest, node:test, and pytest parsers, with recorded-output tests and end-to-end fixture runs for all six runners. Failure paths are relative to the project root in monorepos.
- Pane: ratatui screen with the failure list, streaming output, rerun failed, send to agent, auto-run toggle, raw log popup; control socket so `run` and the idle hook can drive an open pane.
- Scaffold: manifest, data model, adapter registry, go adapter, project detection, config file, prompt formatting, state paths, fixture projects, recorded-output parser tests, house tooling and CI.
