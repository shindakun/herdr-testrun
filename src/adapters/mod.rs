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

/// `path` relative to `root` when it is under it, else unchanged. Accepts
/// `file://` URLs, which node and vitest print in stack frames.
pub fn relative_to(root: &Path, path: &str) -> std::path::PathBuf {
    let path = path.strip_prefix("file://").unwrap_or(path);
    let p = Path::new(path);
    p.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| p.to_path_buf())
}

/// The first stack frame in `text` that points into the project: a
/// `path:line:col` inside `root` and outside `node_modules`. Frames look
/// like `at f (path:1:2)`, `at path:1:2`, or `f (file:///path:1:2)`.
pub fn first_project_frame(text: &str, root: &Path) -> Option<(std::path::PathBuf, u32)> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let loc = match (line.rfind('('), line.ends_with(')')) {
            (Some(open), true) => &line[open + 1..line.len() - 1],
            _ => line.strip_prefix("at ")?,
        };
        let loc = loc.strip_prefix("file://").unwrap_or(loc);
        // path:line:col; the path may hold colons only on Windows.
        let (rest, _col) = loc.rsplit_once(':')?;
        let (path, line_no) = rest.rsplit_once(':')?;
        let line_no: u32 = line_no.parse().ok()?;
        if path.contains("node_modules") || !Path::new(path).starts_with(root) {
            return None;
        }
        Some((relative_to(root, path), line_no))
    })
}

/// Drops stack frames from `node_modules` and node internals so the kept
/// output is the message plus the project's own frames.
pub fn strip_noise_frames(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("at ")
                && (t.contains("node_modules/")
                    || t.contains("(node:")
                    || t.starts_with("at node:")))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_frames() {
        let root = Path::new("/r");
        let text = "Error: boom\n    at Object.<anonymous> (/r/node_modules/x/i.js:1:1)\n    at Object.toBe (/r/src/a.test.js:8:21)\n";
        assert_eq!(
            first_project_frame(text, root),
            Some((std::path::PathBuf::from("src/a.test.js"), 8))
        );
        let text = "AssertionError: x\n    at /r/math.test.js:9:21\n";
        assert_eq!(first_project_frame(text, root).unwrap().1, 9);
        let text = "TestContext.<anonymous> (file:///r/test/m.test.js:10:10)\nTest.run (node:internal/test_runner/test:1397:25)";
        assert_eq!(
            first_project_frame(text, root),
            Some((std::path::PathBuf::from("test/m.test.js"), 10))
        );
        assert_eq!(first_project_frame("at /elsewhere/a.js:1:1", root), None);
    }

    #[test]
    fn noise_frames_are_dropped() {
        let text = "msg\n    at Object.toBe (/r/a.test.js:8:21)\n    at x (/r/node_modules/j/i.js:1:1)\n    at processTicksAndRejections (node:internal/process/task_queues:104:5)";
        assert_eq!(
            strip_noise_frames(text),
            "msg\n    at Object.toBe (/r/a.test.js:8:21)"
        );
    }

    #[test]
    fn relative_paths() {
        assert_eq!(
            relative_to(Path::new("/r"), "/r/a/b.js"),
            std::path::PathBuf::from("a/b.js")
        );
        assert_eq!(
            relative_to(Path::new("/r"), "file:///r/a.js"),
            std::path::PathBuf::from("a.js")
        );
        assert_eq!(
            relative_to(Path::new("/r"), "/x/a.js"),
            std::path::PathBuf::from("/x/a.js")
        );
    }

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
