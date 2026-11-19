//! `--changed-since`: asking git which files changed.

use crate::output::die;
use std::collections::HashSet;
use std::path::PathBuf;

/// `--changed-since`: the files (canonical paths) that differ from git revision `rev`, in
/// the working tree or the index, plus untracked files that aren't ignored, under the
/// working directory. `rev` has been checked not to start with `-`.
pub fn git_changed(rev: &str) -> HashSet<PathBuf> {
    let git = |args: &[&str]| -> Vec<u8> {
        let o = std::process::Command::new("git")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap_or_else(|e| die(format!("--changed-since: running git: {e}")));
        if !o.status.success() {
            die(format!(
                "--changed-since: git {}: {}",
                args[0],
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        o.stdout
    };
    let mut changed = HashSet::new();
    for list in [
        git(&["diff", "--name-only", "-z", "--relative", rev, "--"]),
        git(&["ls-files", "-z", "--others", "--exclude-standard"]),
    ] {
        for name in list.split(|b| *b == 0).filter(|n| !n.is_empty()) {
            #[cfg(unix)]
            let p =
                PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(name));
            #[cfg(not(unix))]
            let p = PathBuf::from(String::from_utf8_lossy(name).into_owned());
            // a file deleted since `rev` has nothing to check
            if let Ok(c) = std::fs::canonicalize(&p) {
                changed.insert(c);
            }
        }
    }
    changed
}
