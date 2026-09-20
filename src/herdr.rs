//! The environment Herdr injects into plugin commands, the context and event
//! JSON it passes, and calls back into Herdr through `HERDR_BIN_PATH`. Names
//! follow herdr 0.9.1.

use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[derive(Debug, Clone)]
pub struct PluginEnv {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub bin_path: PathBuf,
    /// The invocation context, when Herdr passed one.
    pub context: Option<Context>,
}

impl PluginEnv {
    pub fn from_env() -> Result<Self, String> {
        let context = match var("HERDR_PLUGIN_CONTEXT_JSON") {
            Some(json) => Some(Context::parse(&json)?),
            None => None,
        };
        Ok(Self {
            config_dir: var("HERDR_PLUGIN_CONFIG_DIR")
                .map(PathBuf::from)
                .ok_or("HERDR_PLUGIN_CONFIG_DIR is not set; run under herdr")?,
            state_dir: var("HERDR_PLUGIN_STATE_DIR")
                .map(PathBuf::from)
                .ok_or("HERDR_PLUGIN_STATE_DIR is not set; run under herdr")?,
            bin_path: var("HERDR_BIN_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("herdr")),
            context,
        })
    }

    /// Whether this process was started by Herdr.
    pub fn present() -> bool {
        var("HERDR_PLUGIN_STATE_DIR").is_some()
    }

    /// Runs `herdr <args>` and returns stdout.
    pub fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new(&self.bin_path)
            .args(args)
            .output()
            .map_err(|e| format!("spawn {}: {e}", self.bin_path.display()))?;
        if !out.status.success() {
            return Err(format!(
                "herdr {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// `herdr agent list`, parsed.
    pub fn agents(&self) -> Result<Vec<Agent>, String> {
        parse_agent_list(&self.run(&["agent", "list"])?)
    }

    /// `herdr agent prompt TARGET TEXT`. Returns once Herdr has written the
    /// text and Enter; it does not wait for the agent's turn. Fails with
    /// `agent_blocked` when the agent is showing an approval prompt.
    pub fn prompt(&self, target: &str, text: &str) -> Result<(), String> {
        self.run(&["agent", "prompt", target, text]).map(drop)
    }
}

/// The parts of `HERDR_PLUGIN_CONTEXT_JSON` this plugin reads. Pane commands
/// get it too, with the focused pane at the moment the pane opened.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Context {
    pub workspace_id: Option<String>,
    pub workspace_cwd: Option<String>,
    pub focused_pane_id: Option<String>,
    pub focused_pane_cwd: Option<String>,
    pub focused_pane_agent: Option<String>,
    pub focused_pane_status: Option<String>,
    pub invocation_source: Option<String>,
}

impl Context {
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("HERDR_PLUGIN_CONTEXT_JSON: {e}"))
    }

    /// The directory a run should start from: the focused pane's cwd, else
    /// the workspace cwd.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.focused_pane_cwd
            .as_deref()
            .or(self.workspace_cwd.as_deref())
            .map(PathBuf::from)
    }
}

/// One row of `herdr agent list`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Agent {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent_status: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentListing {
    result: AgentList,
}

#[derive(Debug, Deserialize)]
struct AgentList {
    agents: Vec<Agent>,
}

pub fn parse_agent_list(json: &str) -> Result<Vec<Agent>, String> {
    let l: AgentListing = serde_json::from_str(json).map_err(|e| format!("agent list: {e}"))?;
    Ok(l.result.agents)
}

/// The agent `send` targets: the workspace's only agent, else the focused
/// one, else the first. `None` when the workspace has no agent.
pub fn pick_agent<'a>(
    agents: &'a [Agent],
    workspace_id: &str,
    focused_pane: Option<&str>,
) -> Option<&'a Agent> {
    let mine: Vec<&Agent> = agents
        .iter()
        .filter(|a| a.workspace_id == workspace_id)
        .collect();
    match mine.as_slice() {
        [] => None,
        [one] => Some(one),
        many => many
            .iter()
            .find(|a| Some(a.pane_id.as_str()) == focused_pane)
            .or_else(|| many.iter().find(|a| a.focused))
            .copied()
            .or(Some(many[0])),
    }
}

/// `HERDR_PLUGIN_EVENT_JSON` for `pane.agent_status_changed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusEvent {
    pub pane_id: String,
    pub workspace_id: String,
    /// `idle`, `working`, `blocked`, `done`, or `unknown`.
    pub agent_status: String,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    event: String,
    data: EventData,
}

#[derive(Debug, Deserialize)]
struct EventData {
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    agent_status: Option<String>,
}

impl AgentStatusEvent {
    /// `None` when the JSON is some other event.
    pub fn parse(json: &str) -> Result<Option<Self>, String> {
        let e: Envelope =
            serde_json::from_str(json).map_err(|e| format!("HERDR_PLUGIN_EVENT_JSON: {e}"))?;
        if e.event != "pane_agent_status_changed" {
            return Ok(None);
        }
        let missing = |f: &str| format!("HERDR_PLUGIN_EVENT_JSON: missing {f}");
        Ok(Some(Self {
            pane_id: e.data.pane_id.ok_or_else(|| missing("pane_id"))?,
            workspace_id: e.data.workspace_id.ok_or_else(|| missing("workspace_id"))?,
            agent_status: e.data.agent_status.ok_or_else(|| missing("agent_status"))?,
        }))
    }

    /// `idle` and `done` both mean ready for input.
    pub fn is_idle(&self) -> bool {
        matches!(self.agent_status.as_str(), "idle" | "done")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENTS: &str = include_str!("../tests/fixtures/agent_list.json");
    const EVENT: &str = include_str!("../tests/fixtures/agent_status_event.json");

    #[test]
    fn parses_agent_list() {
        let agents = parse_agent_list(AGENTS).unwrap();
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[0].pane_id, "w3:p1");
        assert_eq!(agents[0].workspace_id, "w3");
        assert_eq!(agents[0].agent_status, "idle");
        assert_eq!(agents[0].agent.as_deref(), Some("claude"));
        assert!(agents[1].focused);
    }

    #[test]
    fn picks_the_workspace_agent() {
        let agents = parse_agent_list(AGENTS).unwrap();
        assert_eq!(pick_agent(&agents, "w5", None).unwrap().pane_id, "w5:p1");
        assert!(pick_agent(&agents, "w9", None).is_none());
        let mut two = agents.clone();
        two[0].workspace_id = "w5".into();
        assert_eq!(
            pick_agent(&two, "w5", Some("w3:p1")).unwrap().pane_id,
            "w3:p1"
        );
        assert_eq!(pick_agent(&two, "w5", None).unwrap().pane_id, "w5:p1");
        two[1].focused = false;
        assert_eq!(pick_agent(&two, "w5", None).unwrap().pane_id, "w3:p1");
    }

    #[test]
    fn parses_status_event() {
        let ev = AgentStatusEvent::parse(EVENT).unwrap().unwrap();
        assert_eq!(ev.pane_id, "w3:p1");
        assert_eq!(ev.workspace_id, "w3");
        assert!(ev.is_idle());
        let other = r#"{"event":"pane_focused","data":{"type":"pane_focused","pane_id":"w1:p1","workspace_id":"w1"}}"#;
        assert_eq!(AgentStatusEvent::parse(other).unwrap(), None);
        assert!(AgentStatusEvent::parse("{").is_err());
    }

    #[test]
    fn context_cwd_prefers_the_focused_pane() {
        let c = Context::parse(
            r#"{"workspace_id":"w1","workspace_cwd":"/ws","focused_pane_id":"w1:p2","focused_pane_cwd":"/ws/sub","correlation_id":"x"}"#,
        )
        .unwrap();
        assert_eq!(c.cwd(), Some(PathBuf::from("/ws/sub")));
        let c = Context::parse(r#"{"workspace_cwd":"/ws"}"#).unwrap();
        assert_eq!(c.cwd(), Some(PathBuf::from("/ws")));
        assert_eq!(Context::parse("{}").unwrap().cwd(), None);
    }
}
