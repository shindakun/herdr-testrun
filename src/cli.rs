//! The subcommands. `run` and `send` work with no Herdr present; `pane`
//! does too for development; `on-agent-idle` and `log` need the Herdr
//! environment.

use std::path::{Path, PathBuf};

use crate::herdr::{self, AgentStatusEvent, PluginEnv};
use crate::job::{self, Scope};
use crate::model::{Failure, RunResult};
use crate::sock::{self, Request};
use crate::state::ProjectState;
use crate::{detect, prompt, tui};

pub const USAGE: &str = "usage: herdr-testrun pane [--dir PATH] | run [--dir PATH] [--json] \
| send [--dir PATH] [--print] | log [--dir PATH] | on-agent-idle";

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

/// The project root for a subcommand. `--dir` or `HERDR_TESTRUN_ROOT` is
/// the root as given. Otherwise the detection walk starts at the first of:
/// the focused pane's cwd, the workspace's agent pane cwd, the workspace
/// cwd, the process cwd. A cwd inside Herdr's plugins directory is a
/// plugin pane (a file viewer, this pane) and is skipped.
fn project_root(dir: Option<PathBuf>, env: Option<&PluginEnv>) -> Result<PathBuf, String> {
    if let Some(d) = dir.or_else(herdr::root_override) {
        return std::fs::canonicalize(&d).map_err(|e| format!("{}: {e}", d.display()));
    }
    let start = match env {
        Some(env) => start_dir(env)?,
        None => None,
    };
    let start = match start {
        Some(s) => s,
        None => std::env::current_dir().map_err(|e| format!("cwd: {e}"))?,
    };
    Ok(detect::find_root(&start))
}

fn start_dir(env: &PluginEnv) -> Result<Option<PathBuf>, String> {
    let ctx = env.context.as_ref();
    let usable = |p: Option<&str>| {
        p.map(PathBuf::from)
            .filter(|p| p.is_dir() && !env.is_plugin_dir(p))
    };
    if let Some(p) = usable(ctx.and_then(|c| c.focused_pane_cwd.as_deref())) {
        return Ok(Some(p));
    }
    if let Some(workspace) = ctx.and_then(|c| c.workspace_id.as_deref()) {
        let agents = env.agents()?;
        let agent = herdr::pick_agent(
            &agents,
            workspace,
            ctx.and_then(|c| c.focused_pane_id.as_deref()),
        );
        if let Some(p) = usable(agent.and_then(|a| a.cwd.as_deref())) {
            return Ok(Some(p));
        }
    }
    Ok(usable(ctx.and_then(|c| c.workspace_cwd.as_deref())))
}

/// Under Herdr the plugin state dir; standalone, a directory under temp.
pub fn state_dir(env: Option<&PluginEnv>) -> PathBuf {
    match env {
        Some(e) => e.state_dir.clone(),
        None => std::env::temp_dir().join("herdr-testrun"),
    }
}

/// `pane [--dir PATH]`: the TUI.
pub fn pane(args: &[String]) -> Result<(), String> {
    let a = Args::parse("pane", args, &[])?;
    let env = plugin_env()?;
    let root = project_root(a.dir, env.as_ref())?;
    tui::run(root, env)
}

/// `run [--dir PATH] [--json]`. With a pane open for this root, ask it to
/// run. Under Herdr with no pane, open one; it runs on start. Otherwise run
/// inline and print the failures, exit 1 when any fail.
pub fn run(args: &[String]) -> Result<(), String> {
    let a = Args::parse("run", args, &["--json"])?;
    let env = plugin_env()?;
    let root = project_root(a.dir.clone(), env.as_ref())?;
    let state = ProjectState::open(&state_dir(env.as_ref()), &root)?;
    let json = a.flag("--json");

    let socket = state.dir.join(sock::FILE);
    if sock::probe(&socket) {
        let reply = sock::send(&socket, &Request::Run)?;
        return print_reply(&reply, json);
    }
    if let Some(env) = &env {
        open_pane(env, &root)?;
        println!("opened the Tests pane for {}", root.display());
        return Ok(());
    }

    let results = job::run(&root, None, &state, &Scope::All, |_| {})?;
    if json {
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

fn print_reply(reply: &sock::Response, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(reply).map_err(|e| e.to_string())?
        );
    } else if !reply.message.is_empty() {
        println!("{}", reply.message);
    }
    if reply.ok {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

/// `herdr plugin pane open` for the Tests pane at `root`.
fn open_pane(env: &PluginEnv, root: &Path) -> Result<(), String> {
    let root_env = format!("HERDR_TESTRUN_ROOT={}", root.display());
    let cwd = root.display().to_string();
    env.run(&[
        "plugin",
        "pane",
        "open",
        "--plugin",
        &herdr::plugin_id(),
        "--entrypoint",
        "tests",
        "--cwd",
        &cwd,
        "--env",
        &root_env,
        "--focus",
    ])
    .map(drop)
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
    let failures: Vec<Failure> = results
        .iter()
        .flat_map(|r| r.failures.iter().cloned())
        .collect();
    if failures.is_empty() {
        return Err("last run had no failures".into());
    }
    if a.flag("--print") {
        print!("{}", prompt::format(&failures));
        return Ok(());
    }
    let env = env.ok_or("send needs the Herdr environment; use --print outside Herdr")?;
    println!("{}", send_failures(&env, &failures)?);
    Ok(())
}

/// Prompts the workspace's agent with `failures`. Returns the line to show.
pub fn send_failures(env: &PluginEnv, failures: &[Failure]) -> Result<String, String> {
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
    env.prompt(&agent.pane_id, &prompt::format(failures))?;
    Ok(format!(
        "sent {} failure{} to {}",
        failures.len(),
        if failures.len() == 1 { "" } else { "s" },
        agent.pane_id
    ))
}

/// `log [--dir PATH]`: page the last run's raw output. Backs the `log`
/// popup pane; `o` in the Tests pane opens it with `HERDR_TESTRUN_ROOT` set.
pub fn log(args: &[String]) -> Result<(), String> {
    let a = Args::parse("log", args, &[])?;
    let env = plugin_env()?;
    let root = project_root(a.dir, env.as_ref())?;
    let state = ProjectState::open(&state_dir(env.as_ref()), &root)?;
    let log = state.raw_log();
    if !log.is_file() {
        return Err(format!("no raw log for {}; run first", root.display()));
    }
    let pager = std::env::var("PAGER")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "less".to_string());
    let mut parts = pager.split_whitespace();
    let program = parts.next().unwrap_or("less");
    let mut cmd = std::process::Command::new(program);
    cmd.args(parts);
    if program == "less" {
        // Raw control chars through, start at the end.
        cmd.args(["-R", "+G"]);
    }
    cmd.arg(&log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        return Err(format!("exec {program}: {err}"));
    }
    #[allow(unreachable_code)]
    {
        let status = cmd.status().map_err(|e| format!("{program}: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{program} exited with {status}"))
        }
    }
}

/// `on-agent-idle`: the `pane.agent_status_changed` hook. When the status
/// is idle, auto-run is on for the pane's project, and a Tests pane is
/// open, send the pane `auto-run`. The pane applies the change gate and
/// the auto-send limits.
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
    let socket = state.dir.join(sock::FILE);
    if !sock::probe(&socket) {
        return Ok(());
    }
    let reply = sock::send(&socket, &Request::AutoRun)?;
    println!("{}: {}", root.display(), reply.message);
    Ok(())
}

pub fn print_result(r: &RunResult, root: &Path) {
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
