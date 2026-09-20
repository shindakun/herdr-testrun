//! Per-project state under `HERDR_PLUGIN_STATE_DIR/<sha of root path>/`:
//! `last.json`, `raw.log`, `settings.json`, and adapter result files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::RunResult;

pub const LAST_FILE: &str = "last.json";
pub const RAW_LOG: &str = "raw.log";
pub const SETTINGS_FILE: &str = "settings.json";

/// What the pane remembers per worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Rerun when the workspace's agent goes idle.
    pub auto_run: bool,
    /// After an auto run with failures, send them to the agent.
    pub auto_send: bool,
    /// Auto sends allowed per session before stopping.
    pub max_rounds: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_run: false,
            auto_send: false,
            max_rounds: 3,
        }
    }
}

/// The first 16 hex chars of the SHA-256 of the root path.
pub fn root_hash(root: &Path) -> String {
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    let mut s = String::with_capacity(16);
    for b in &digest[..8] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The state directory for `root`, created.
pub struct ProjectState {
    pub dir: PathBuf,
}

impl ProjectState {
    pub fn open(state_dir: &Path, root: &Path) -> Result<Self, String> {
        let dir = state_dir.join(root_hash(root));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn raw_log(&self) -> PathBuf {
        self.dir.join(RAW_LOG)
    }

    pub fn settings(&self) -> Result<Settings, String> {
        read_json(&self.dir.join(SETTINGS_FILE)).map(Option::unwrap_or_default)
    }

    #[allow(dead_code)] // the pane's `w` key writes settings
    pub fn save_settings(&self, s: &Settings) -> Result<(), String> {
        write_json(&self.dir.join(SETTINGS_FILE), s)
    }

    pub fn last(&self) -> Result<Option<Vec<RunResult>>, String> {
        read_json(&self.dir.join(LAST_FILE))
    }

    pub fn save_last(&self, results: &[RunResult]) -> Result<(), String> {
        write_json(&self.dir.join(LAST_FILE), &results)
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes to a sibling temp file, then renames, so a reader never sees a
/// partial file.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AdapterId, Failure, RerunKey};

    #[test]
    fn hash_is_stable_and_short() {
        let h = root_hash(Path::new("/Users/x/proj"));
        assert_eq!(h.len(), 16);
        assert_eq!(h, root_hash(Path::new("/Users/x/proj")));
        assert_ne!(h, root_hash(Path::new("/Users/x/proj2")));
    }

    #[test]
    fn settings_round_trip() {
        let base = crate::testutil::tempdir("state-settings");
        let st = ProjectState::open(&base, Path::new("/p")).unwrap();
        assert_eq!(st.settings().unwrap(), Settings::default());
        let s = Settings {
            auto_run: true,
            auto_send: true,
            max_rounds: 5,
        };
        st.save_settings(&s).unwrap();
        assert_eq!(st.settings().unwrap(), s);
    }

    #[test]
    fn last_round_trip() {
        let base = crate::testutil::tempdir("state-last");
        let st = ProjectState::open(&base, Path::new("/p")).unwrap();
        assert!(st.last().unwrap().is_none());
        let mut r = RunResult::new("go", Path::new("/p"), 1);
        r.failures.push(Failure {
            adapter: AdapterId("go"),
            name: "TestX".into(),
            file: None,
            line: None,
            output: String::new(),
            rerun: RerunKey::GoTest {
                pkg: "m".into(),
                name: "TestX".into(),
            },
        });
        r.failed = 1;
        st.save_last(&[r]).unwrap();
        let back = st.last().unwrap().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].adapter, "go");
        assert_eq!(back[0].failures[0].name, "TestX");
    }
}
