//! `.herdr-testrun.toml` at the project root, or
//! `HERDR_PLUGIN_CONFIG_DIR/<sha of root path>.toml`. When it exists it
//! replaces marker detection.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const FILE_NAME: &str = ".herdr-testrun.toml";

/// Seconds before a run is killed. Overridable per project.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub timeout_secs: u64,
    #[serde(rename = "target")]
    pub targets: Vec<TargetConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    /// An adapter id from the registry.
    pub adapter: String,
    /// Relative to the project root.
    #[serde(default = "dot")]
    pub dir: PathBuf,
    /// Replaces the adapter's full-run command. The adapter still parses.
    #[serde(default)]
    pub command: Option<Vec<String>>,
}

fn dot() -> PathBuf {
    PathBuf::from(".")
}

impl Config {
    /// The project's config file if one exists, or `None`.
    pub fn load(root: &Path, config_dir: Option<&Path>) -> Result<Option<Self>, String> {
        let mut candidates = vec![root.join(FILE_NAME)];
        if let Some(dir) = config_dir {
            candidates.push(dir.join(format!("{}.toml", crate::state::root_hash(root))));
        }
        for path in candidates {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    return Self::parse(&text)
                        .map(Some)
                        .map_err(|e| format!("{}: {e}", path.display()))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Ok(None)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        for t in &c.targets {
            if crate::adapters::by_id(&t.adapter).is_none() {
                return Err(format!("unknown adapter `{}`", t.adapter));
            }
            if t.command.as_ref().is_some_and(|c| c.is_empty()) {
                return Err(format!("target `{}`: command must not be empty", t.adapter));
            }
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_default() {
        let c = Config::parse("").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.timeout_secs, 600);
    }

    #[test]
    fn reads_targets() {
        let c = Config::parse(
            r#"
timeout_secs = 30

[[target]]
adapter = "cargo"
command = ["cargo", "nextest", "run"]

[[target]]
adapter = "jest"
dir = "web"
"#,
        )
        .unwrap();
        assert_eq!(c.timeout_secs, 30);
        assert_eq!(c.targets.len(), 2);
        assert_eq!(c.targets[0].dir, PathBuf::from("."));
        assert_eq!(
            c.targets[0].command.as_deref(),
            Some(&["cargo".to_string(), "nextest".into(), "run".into()][..])
        );
        assert_eq!(c.targets[1].dir, PathBuf::from("web"));
        assert!(c.targets[1].command.is_none());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Config::parse("[[target]]\nadapter = \"mocha\"\n").is_err());
        assert!(Config::parse("[[target]]\nadapter = \"go\"\ncommand = []\n").is_err());
        assert!(Config::parse("colour = 1\n").is_err());
        assert!(Config::parse("[[target]]\nadapter = \"go\"\nextra = 1\n").is_err());
    }

    #[test]
    fn load_prefers_the_project_file() {
        let root = crate::testutil::tempdir("config-root");
        let cfg = crate::testutil::tempdir("config-dir");
        assert_eq!(Config::load(&root, Some(&cfg)).unwrap(), None);
        let hashed = cfg.join(format!("{}.toml", crate::state::root_hash(&root)));
        std::fs::write(&hashed, "timeout_secs = 5\n").unwrap();
        assert_eq!(
            Config::load(&root, Some(&cfg))
                .unwrap()
                .unwrap()
                .timeout_secs,
            5
        );
        std::fs::write(root.join(FILE_NAME), "timeout_secs = 7\n").unwrap();
        assert_eq!(
            Config::load(&root, Some(&cfg))
                .unwrap()
                .unwrap()
                .timeout_secs,
            7
        );
    }
}
