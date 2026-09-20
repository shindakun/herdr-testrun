//! The change gate: a hash of the worktree's state, so an automatic rerun
//! is skipped when nothing changed since the last run.

use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

/// A hash of `git status --porcelain`, `git diff HEAD`, and the size and
/// mtime of every untracked file. `None` when `root` is not in a git
/// worktree; callers then run every time.
pub fn change_hash(root: &Path) -> Option<String> {
    let status = git(root, &["status", "--porcelain", "--untracked-files=all"])?;
    let diff = git(root, &["diff", "HEAD", "--no-color", "--no-ext-diff"]).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(status.as_bytes());
    hasher.update(diff.as_bytes());
    for line in status.lines() {
        if let Some(path) = line.strip_prefix("?? ") {
            hasher.update(path.as_bytes());
            if let Ok(meta) = std::fs::metadata(root.join(path)) {
                hasher.update(meta.len().to_le_bytes());
                if let Ok(t) = meta.modified() {
                    if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                        hasher.update(d.as_nanos().to_le_bytes());
                    }
                }
            }
        }
    }
    let digest = hasher.finalize();
    Some(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tempdir;

    fn sh(root: &Path, cmd: &str) {
        let ok = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(root)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "{cmd}");
    }

    #[test]
    fn no_repo_is_none() {
        assert_eq!(change_hash(&tempdir("gate-none")), None);
    }

    #[test]
    fn tracks_edits_untracked_files_and_staging() {
        let root = tempdir("gate-repo");
        sh(
            &root,
            "git init -q && git -c user.email=t@t -c user.name=t commit -q --allow-empty -m init",
        );
        let clean = change_hash(&root).unwrap();
        assert_eq!(
            change_hash(&root).unwrap(),
            clean,
            "stable when nothing changes"
        );

        std::fs::write(root.join("new.txt"), "a").unwrap();
        let untracked = change_hash(&root).unwrap();
        assert_ne!(untracked, clean, "an untracked file changes the hash");

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("new.txt"), "ab").unwrap();
        let edited = change_hash(&root).unwrap();
        assert_ne!(
            edited, untracked,
            "editing an untracked file changes the hash"
        );

        sh(&root, "git add new.txt");
        let staged = change_hash(&root).unwrap();
        assert_ne!(staged, edited, "staging changes the hash");

        sh(
            &root,
            "git -c user.email=t@t -c user.name=t commit -q -m add",
        );
        std::fs::write(root.join("new.txt"), "abc").unwrap();
        let modified = change_hash(&root).unwrap();
        assert_ne!(modified, staged, "a tracked edit changes the hash");
    }
}
