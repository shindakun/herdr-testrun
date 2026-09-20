//! The one-shot subcommands. `run` works with no Herdr present; `send` and
//! `on-agent-idle` need the Herdr environment.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{self, Config};
use crate::detect::{self, Target};
use crate::herdr::{self, AgentStatusEvent, PluginEnv};
use crate::model::RunResult;
use crate::state::ProjectState;
use crate::{adapters, prompt, runner};

pub const USAGE: &str =
    "usage: herdr-testrun pane | run [--dir PATH] [--json] | send [--dir PATH] [--print] | on-agent-idle";

/// `--dir PATH` plus boolean flags from an argv slice.
struct Args {
    dir: Option<PathBuf>,
    flags: Vec<String>,
}

impl Args {
    fn parse(cmd: &str, args: &[String], allowed: &[&str]) -> Result<Self, String> {
        let mut a = Args {
            dir: None,
            flags: Vec::new(),
        };
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--dir" => a.dir = Some(PathBuf::from(it.next().ok_or("--dir needs a value")?)),
                f if allowed.contains(&f) => a.flags.push(f.to_string()),
                other => return Err(format!("{cmd}: unknown argument {other}\n{USAGE}")),
            }
        }
        Ok(a)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }
}

/// The Herdr environment when this process runs under Herdr.
fn plugin_env() -> Result<Option<PluginEnv>, String> {
    if PluginEnv::present() {
        PluginEnv::from_env().map(Some)
    } else {
        Ok(None)
    }
}

/// The project root for a subcommand. `--dir` is the root as given. With no
/// `--dir`, the detection walk starts at the focused pane's cwd from the
/// Herdr context, else the process cwd.
fn project_root(dir: Option<PathBuf>, env: Option<&PluginEnv>) -> Result<PathBuf, String> {
    if let Some(d) = dir {
        return std::fs::canonicalize(&d).map_err(|e| format!("{}: {e}", d.display()));
    }
    let start = match env.and_then(|e| e.context.as_ref()).and_then(|c| c.cwd()) {
        Some(c) => c,
        None => std::env::current_dir().map_err(|e| format!("cwd: {e}"))?,
    };
    Ok(detect::find_root(&start))
}

/// Under Herdr the plugin state dir; standalone, a directory under temp.
fn state_dir(env: Option<&PluginEnv>) -> PathBuf {
    match env {
        Some(e) => e.state_dir.clone(),
        None => std::env::temp_dir().join("herdr-testrun"),
    }
}

/// `run [--dir PATH] [--json]`: detect, run every target once, print the
/// failures. Works with no Herdr present; under Herdr it starts from the
/// focused pane's cwd and keeps `last.json` and `raw.log` in the state dir.
pub fn run(args: &[String]) -> Result<(), String> {
    let a = Args::parse("run", args, &["--json"])?;
    let env = plugin_env()?;
    let root = project_root(a.dir.clone(), env.as_ref())?;
    let results = run_root(&root, env.as_ref())?;
    if a.flag("--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?
        );
    } else {
        for r in &results {
            print_result(r, &root);
        }
    }
    if results.iter().all(RunResult::ok) {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

/// Detects the targets under `root`, runs each once, and records
/// `last.json` and `raw.log`.
pub fn run_root(root: &Path, env: Option<&PluginEnv>) -> Result<Vec<RunResult>, String> {
    let config = Config::load(root, env.map(|e| e.config_dir.as_path()))?;
    let targets = detect::targets(root, config.as_ref())?;
    if targets.is_empty() {
        return Err(format!("no test runner found under {}", root.display()));
    }
    let timeout = Duration::from_secs(
        config
            .as_ref()
            .map_or(config::DEFAULT_TIMEOUT_SECS, |c| c.timeout_secs),
    );
    let state = ProjectState::open(&state_dir(env), root)?;

    let mut results = Vec::new();
    let mut raw = String::new();
    for target in &targets {
        let (result, log) = run_target(target, &state.dir, timeout)?;
        raw.push_str(&log);
        results.push(result);
    }
    std::fs::write(state.raw_log(), &raw)
        .map_err(|e| format!("{}: {e}", state.raw_log().display()))?;
    for r in &mut results {
        r.raw_log = state.raw_log();
    }
    state.save_last(&results)?;
    Ok(results)
}

/// `send [--dir PATH] [--print]`: the last run's failures as one prompt to
/// the workspace's agent. `--print` writes the prompt to stdout instead.
pub fn send(args: &[String]) -> Result<(), String> {
    let a = Args::parse("send", args, &["--print"])?;
    let env = plugin_env()?;
    let root = project_root(a.dir.clone(), env.as_ref())?;
    let state = ProjectState::open(&state_dir(env.as_ref()), &root)?;
    let results = state
        .last()?
        .ok_or_else(|| format!("no recorded run for {}; run first", root.display()))?;
    let failures: Vec<_> = results
        .iter()
        .flat_map(|r| r.failures.iter().cloned())
        .collect();
    if failures.is_empty() {
        return Err("last run had no failures".into());
    }
    let text = prompt::format(&failures);
    if a.flag("--print") {
        print!("{text}");
        return Ok(());
    }
    let env = env.ok_or("send needs the Herdr environment; use --print outside Herdr")?;
    let ctx = env.context.as_ref();
    let workspace = ctx
        .and_then(|c| c.workspace_id.as_deref())
        .ok_or("no workspace in HERDR_PLUGIN_CONTEXT_JSON")?;
    let agents = env.agents()?;
    let agent = herdr::pick_agent(
        &agents,
        workspace,
        ctx.and_then(|c| c.focused_pane_id.as_deref()),
    )
    .ok_or_else(|| format!("no agent in workspace {workspace}"))?;
    env.prompt(&agent.pane_id, &text)?;
    println!("sent {} failure(s) to {}", failures.len(), agent.pane_id);
    Ok(())
}

/// `on-agent-idle`: the `pane.agent_status_changed` hook. Exits quietly
/// unless the status is idle and auto-run is on for the pane's project. The
/// guarded rerun itself lives in the pane (docs/PLAN.md build order, step 6).
pub fn on_agent_idle() -> Result<(), String> {
    let json = std::env::var("HERDR_PLUGIN_EVENT_JSON")
        .map_err(|_| "HERDR_PLUGIN_EVENT_JSON is not set; run under herdr")?;
    let Some(event) = AgentStatusEvent::parse(&json)? else {
        return Ok(());
    };
    if !event.is_idle() {
        return Ok(());
    }
    let env = PluginEnv::from_env()?;
    let root = project_root(None, Some(&env))?;
    let state = ProjectState::open(&env.state_dir, &root)?;
    if !state.settings()?.auto_run {
        return Ok(());
    }
    Err(format!(
        "auto-run is on for {} but the pane is not built yet; see docs/PLAN.md build order, step 6",
        root.display()
    ))
}

pub fn run_target(
    target: &Target,
    state: &Path,
    timeout: Duration,
) -> Result<(RunResult, String), String> {
    let mut cmd = match &target.command {
        Some(argv) => {
            let mut c = adapters::base_command(&argv[0], &target.dir);
            c.args(&argv[1..]);
            c
        }
        None => target.adapter.command(&target.dir, state, None),
    };
    let started = std::time::SystemTime::now();
    let out = runner::run(&mut cmd, timeout)?;
    let mut result = target
        .adapter
        .parse(&target.dir, state, &out.stdout, &out.stderr, out.code);
    result.started = started;
    result.duration = out.duration;
    if out.timed_out {
        result.build_error = Some(format!("timed out after {}s", timeout.as_secs()));
    }
    let log = format!(
        "==> {} in {}\n{}{}",
        target.adapter.id(),
        target.dir.display(),
        out.stdout,
        out.stderr
    );
    Ok((result, log))
}

fn print_result(r: &RunResult, root: &Path) {
    let dir = r.root.strip_prefix(root).unwrap_or(&r.root);
    let dir = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    println!(
        "{}: {} failed, {} passed, {} skipped in {:.1}s ({dir})",
        r.adapter,
        r.failed,
        r.passed,
        r.skipped,
        r.duration.as_secs_f64()
    );
    if let Some(err) = &r.build_error {
        println!("  build error:");
        for line in err.lines() {
            println!("    {line}");
        }
    }
    for f in &r.failures {
        let loc = match (&f.file, f.line) {
            (Some(file), Some(line)) => format!(" {}:{line}", file.display()),
            (Some(file), None) => format!(" {}", file.display()),
            _ => String::new(),
        };
        println!("  ✗ {}{loc}", f.name);
        for line in f.output.lines() {
            println!("    {line}");
        }
    }
}
