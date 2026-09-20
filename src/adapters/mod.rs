//! One adapter per test runner. Adding a runner means one file here, one
//! `fixtures/<name>-basic` project, and one recorded output in `tests/output`.

use std::path::Path;
use std::process::Command;

use crate::model::Adapter;

pub mod cargo;
pub mod go;
pub mod jest;
pub mod nodetest;
pub mod pytest;
pub mod vitest;

pub static ALL: &[&dyn Adapter] = &[
    &go::Go,
    &cargo::Cargo,
    &jest::Jest,
    &vitest::Vitest,
    &nodetest::NodeTest,
    &pytest::Pytest,
];

pub fn by_id(id: &str) -> Option<&'static dyn Adapter> {
    ALL.iter().copied().find(|a| a.id() == id)
}

/// The registry's static copy of `id`, if it names an adapter.
pub fn intern(id: &str) -> Option<&'static str> {
    by_id(id).map(|a| a.id())
}

/// Adapters whose `detect` matches `root`, in registry order.
pub fn detect_all(root: &Path) -> Vec<&'static dyn Adapter> {
    ALL.iter().copied().filter(|a| a.detect(root)).collect()
}

/// The `build_error` an adapter reports until its parser lands.
pub fn not_built(id: &str) -> String {
    format!("{id} parser not built yet; see docs/PLAN.md build order")
}

/// A command with cwd set to `root` and stdin closed, the shape every
/// adapter's `command` returns.
pub fn base_command(program: &str, root: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(root).stdin(std::process::Stdio::null());
    cmd
}

/// Reads the package.json at `root`, or `None` when there is none or it does
/// not parse.
pub fn package_json(root: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn has_dependency(pkg: &serde_json::Value, name: &str) -> bool {
    ["dependencies", "devDependencies"]
        .iter()
        .any(|k| pkg.get(k).and_then(|d| d.get(name)).is_some())
}

/// The JS runner prefix for `root`, picked from the lockfile that exists:
/// `bun x`, `pnpm exec`, `yarn`, else `npx`.
pub fn js_runner(root: &Path) -> Vec<&'static str> {
    if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
        vec!["bun", "x"]
    } else if root.join("pnpm-lock.yaml").exists() {
        vec!["pnpm", "exec"]
    } else if root.join("yarn.lock").exists() {
        vec!["yarn"]
    } else {
        vec!["npx"]
    }
}

/// `program args...` with cwd `root`, from a runner prefix plus the tool.
pub fn js_command(root: &Path, tool: &str) -> Command {
    let runner = js_runner(root);
    let mut cmd = base_command(runner[0], root);
    cmd.args(&runner[1..]).arg(tool);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_interned() {
        let mut ids: Vec<&str> = ALL.iter().map(|a| a.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ALL.len());
        assert_eq!(intern("go"), Some("go"));
        assert_eq!(intern("nope"), None);
    }

    #[test]
    fn js_runner_prefers_the_lockfile() {
        let dir = crate::testutil::tempdir("js-runner");
        assert_eq!(js_runner(&dir), vec!["npx"]);
        std::fs::write(dir.join("yarn.lock"), "").unwrap();
        assert_eq!(js_runner(&dir), vec!["yarn"]);
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(js_runner(&dir), vec!["pnpm", "exec"]);
        std::fs::write(dir.join("bun.lockb"), "").unwrap();
        assert_eq!(js_runner(&dir), vec!["bun", "x"]);
    }
}
