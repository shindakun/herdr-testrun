# Fixtures

Recorded from Herdr 0.9.1 on macOS.

- `agent_list.json`: `herdr agent list`, trimmed to three agents. The command prints JSON without a flag; there is no `--json`.
- `agent_status_event.json`: the `HERDR_PLUGIN_EVENT_JSON` an event hook receives for `pane.agent_status_changed`. Built from the 0.9.1 schema (`EventEnvelope` serialized with `EventKind` in snake_case and `EventData` tagged by `type`), not captured from a live hook.
