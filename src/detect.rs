//! Finding the project root and the test targets in it.
//!
//! 1. Walk up from the start directory to the first one holding
//!    `.herdr-testrun.toml` or `.git`. That is the root.
//! 2. A config file names the targets outright.
//! 3. Otherwise every adapter whose markers match the root is a target.
//! 4. A root with no markers is checked one level down, so `web/package.json`
//!    under a Go root is found.

use std::path::{Path, PathBuf};

use crate::adapters;
use crate::config::Config;
use crate::model::Adapter;

/// One thing to run: an adapter in a directory, with an optional command
/// override from the config file.
pub struct Target {
    pub adapter: &'static dyn Adapter,
    /// Absolute.
    pub dir: PathBuf,
    pub command: Option<Vec<String>>,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("adapter", &self.adapter.id())
            .field("dir", &self.dir)
            .field("command", &self.command)
            .finish()
    }
}

/// Directories never searched one level down.
const SKIP_DIRS: &[&str] = &["node_modules", "target", "vendor", "dist", "build"];

/// The nearest ancestor of `start` (inclusive) with a config file or `.git`,
/// else `start` itself.
pub fn find_root(start: &Path) -> PathBuf {
    let start = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    for dir in start.ancestors() {
        if dir.join(crate::config::FILE_NAME).is_file() || dir.join(".git").exists() {
            return dir.to_path_buf();
        }
    }
    start
}

/// The targets for `root`. `config` is the project's config file when it has
/// one.
pub fn targets(root: &Path, config: Option<&Config>) -> Result<Vec<Target>, String> {
    if let Some(cfg) = config {
        return cfg
            .targets
            .iter()
            .map(|t| {
                let adapter = adapters::by_id(&t.adapter)
                    .ok_or_else(|| format!("unknown adapter `{}`", t.adapter))?;
                Ok(Target {
                    adapter,
                    dir: root.join(&t.dir),
                    command: t.command.clone(),
                })
            })
            .collect();
    }
    let mut found: Vec<Target> = adapters::detect_all(root)
        .into_iter()
        .map(|adapter| Target {
            adapter,
            dir: root.to_path_buf(),
            command: None,
        })
        .collect();
    if found.is_empty() {
        for dir in subdirs(root) {
            for adapter in adapters::detect_all(&dir) {
                found.push(Target {
                    adapter,
                    dir: dir.clone(),
                    command: None,
                });
            }
        }
    }
    Ok(found)
}

/// Immediate, non-hidden, non-build subdirectories of `root`, sorted.
fn subdirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref())
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tempdir;

    fn ids(targets: &[Target]) -> Vec<(&'static str, PathBuf)> {
        targets
            .iter()
            .map(|t| (t.adapter.id(), t.dir.clone()))
            .collect()
    }

    #[test]
    fn root_is_the_nearest_git_or_config_dir() {
        let top = tempdir("detect-root");
        let top = std::fs::canonicalize(&top).unwrap();
        let deep = top.join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(find_root(&deep), deep);
        std::fs::create_dir(top.join(".git")).unwrap();
        assert_eq!(find_root(&deep), top);
        std::fs::write(top.join("a").join(crate::config::FILE_NAME), "").unwrap();
        assert_eq!(find_root(&deep), top.join("a"));
    }

    #[test]
    fn markers_at_root() {
        let root = tempdir("detect-markers");
        std::fs::write(root.join("go.mod"), "module x\n").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        assert_eq!(
            ids(&targets(&root, None).unwrap()),
            vec![("go", root.clone()), ("cargo", root.clone())]
        );
    }

    #[test]
    fn one_level_down_when_root_has_none() {
        let root = tempdir("detect-sub");
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/jest")).unwrap();
        std::fs::write(root.join("node_modules/jest/go.mod"), "module x\n").unwrap();
        std::fs::write(
            root.join("web/package.json"),
            r#"{"devDependencies":{"jest":"^29"}}"#,
        )
        .unwrap();
        assert_eq!(
            ids(&targets(&root, None).unwrap()),
            vec![("jest", root.join("web"))]
        );
    }

    #[test]
    fn root_markers_stop_the_subdir_scan() {
        let root = tempdir("detect-root-wins");
        std::fs::write(root.join("go.mod"), "module x\n").unwrap();
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::write(
            root.join("web/package.json"),
            r#"{"devDependencies":{"jest":"^29"}}"#,
        )
        .unwrap();
        assert_eq!(
            ids(&targets(&root, None).unwrap()),
            vec![("go", root.clone())]
        );
    }

    #[test]
    fn config_replaces_detection() {
        let root = tempdir("detect-config");
        std::fs::write(root.join("go.mod"), "module x\n").unwrap();
        let cfg = Config::parse(
            "[[target]]\nadapter = \"cargo\"\ncommand = [\"cargo\", \"nextest\", \"run\"]\n\n[[target]]\nadapter = \"jest\"\ndir = \"web\"\n",
        )
        .unwrap();
        let t = targets(&root, Some(&cfg)).unwrap();
        assert_eq!(
            ids(&t),
            vec![("cargo", root.join(".")), ("jest", root.join("web"))]
        );
        assert_eq!(t[0].command.as_ref().unwrap()[1], "nextest");
    }
}
