//! One run of a project: detect the targets, run each, record the result.
//! Shared by the `run` subcommand and the pane's worker.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::adapters;
use crate::config::{self, Config};
use crate::detect::{self, Target};
use crate::herdr::PluginEnv;
use crate::model::{Failure, RerunKey, RunResult};
use crate::runner::{self, Stream};
use crate::state::ProjectState;

/// What to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    /// Only these failures. Targets with no key are skipped; a target with
    /// a command override runs the override in full, since the override
    /// has no rerun form.
    Only(Vec<Failure>),
}

/// Runs `root`'s targets and writes `last.json` and `raw.log` into its state
/// directory. `on_line` sees every output line as it arrives.
pub fn run(
    root: &Path,
    env: Option<&PluginEnv>,
    state: &ProjectState,
    scope: &Scope,
    mut on_line: impl FnMut(&str),
) -> Result<Vec<RunResult>, String> {
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
    let keys = match scope {
        Scope::All => BTreeMap::new(),
        Scope::Only(failures) => keys_by_adapter(failures),
    };
    let mut results = Vec::new();
    let mut raw = String::new();
    for target in &targets {
        let only: Option<&[RerunKey]> = match scope {
            Scope::All => None,
            Scope::Only(_) => match keys.get(target.adapter.id()) {
                Some(k) => Some(k),
                None => continue,
            },
        };
        let header = format!("==> {} in {}\n", target.adapter.id(), target.dir.display());
        on_line(&header);
        raw.push_str(&header);
        let result = run_target(root, target, &state.dir, timeout, only, |_, line| {
            on_line(line);
            raw.push_str(line);
        })?;
        results.push(result);
    }
    let log = state.raw_log();
    std::fs::write(&log, &raw).map_err(|e| format!("{}: {e}", log.display()))?;
    for r in &mut results {
        r.raw_log = log.clone();
    }
    state.save_last(&results)?;
    Ok(results)
}

/// Runs one target and parses the result. `only` narrows the run to those
/// keys; a command override ignores it. Failure paths come back relative to
/// `root`, not the target dir.
pub fn run_target(
    root: &Path,
    target: &Target,
    state: &Path,
    timeout: Duration,
    only: Option<&[RerunKey]>,
    on_line: impl FnMut(Stream, &str),
) -> Result<RunResult, String> {
    let mut cmd = match &target.command {
        Some(argv) => {
            let mut c = adapters::base_command(&argv[0], &target.dir);
            c.args(&argv[1..]);
            c
        }
        None => target.adapter.command(&target.dir, state, only),
    };
    let started = std::time::SystemTime::now();
    let out = runner::run_with(&mut cmd, timeout, on_line)?;
    let mut result = target
        .adapter
        .parse(&target.dir, state, &out.stdout, &out.stderr, out.code);
    result.started = started;
    result.duration = out.duration;
    if out.timed_out {
        result.build_error = Some(format!("timed out after {}s", timeout.as_secs()));
    }
    if let Ok(sub) = target.dir.strip_prefix(root) {
        if !sub.as_os_str().is_empty() {
            for f in &mut result.failures {
                if let Some(file) = f.file.as_mut().filter(|p| p.is_relative()) {
                    *file = sub.join(&*file);
                }
            }
        }
    }
    Ok(result)
}

fn keys_by_adapter(failures: &[Failure]) -> BTreeMap<&str, Vec<RerunKey>> {
    let mut map: BTreeMap<&str, Vec<RerunKey>> = BTreeMap::new();
    for f in failures {
        map.entry(&f.adapter).or_default().push(f.rerun.clone());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AdapterId;
    use crate::testutil::tempdir;

    #[test]
    fn keys_group_by_adapter() {
        let f = |a: &'static str, n: &str| Failure {
            adapter: AdapterId(a),
            name: n.into(),
            file: None,
            line: None,
            output: String::new(),
            rerun: RerunKey::CargoTest { name: n.into() },
        };
        let failures = [f("cargo", "a"), f("go", "b"), f("cargo", "c")];
        let keys = keys_by_adapter(&failures);
        assert_eq!(keys["cargo"].len(), 2);
        assert_eq!(keys["go"].len(), 1);
    }

    #[test]
    fn override_runs_the_given_command_and_streams() {
        let root = tempdir("job-override");
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        let target = Target {
            adapter: adapters::by_id("cargo").unwrap(),
            dir: root.clone(),
            command: Some(vec!["sh".into(), "-c".into(), "echo hi; exit 101".into()]),
        };
        let mut lines = Vec::new();
        let r = run_target(
            &root,
            &target,
            &root,
            Duration::from_secs(5),
            None,
            |_, l| lines.push(l.to_string()),
        )
        .unwrap();
        assert_eq!(lines, vec!["hi\n"]);
        assert_eq!(r.exit_code, 101);
    }

    #[test]
    fn no_targets_is_an_error() {
        let root = tempdir("job-empty");
        let state = ProjectState::open(&tempdir("job-empty-state"), &root).unwrap();
        let err = run(&root, None, &state, &Scope::All, |_| {}).unwrap_err();
        assert!(err.starts_with("no test runner found"), "{err}");
    }
}
