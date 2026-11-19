//! The files a run processes: the directory walk, each file's config, and running work
//! over them on all cores.

use crate::config::{self, Config, Resolver};
use crate::display_path;
use crate::output::{die, shown};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Set when a directory couldn't be read during the walk, or an argument was refused; the
/// run then exits 2.
pub static WALK_FAILED: AtomicBool = AtomicBool::new(false);
/// `-v`: report each directory a walk skips.
pub static WALK_VERBOSE: AtomicBool = AtomicBool::new(false);

/// The config governing each file to process.
pub type Configs = HashMap<PathBuf, Arc<Config>>;

/// Paths named on the command line are taken as given (a named symlink is followed).
/// Below them, symlinks are never followed, so a link cycle can't loop the walk and a link
/// out of the tree can't make us read or rewrite files elsewhere; only regular files count.
/// Whether a path is excluded is up to the config governing its directory (`dir_cfg`, found
/// here for an argument), so a nested config is never read when its parent's excludes the
/// directory holding it. A config that failed to load is fatal only once something needs it.
fn walk(
    p: &Path,
    dir_cfg: Option<&Result<Arc<Config>, String>>,
    out: &mut Vec<(PathBuf, Arc<Config>)>,
    res: &mut Resolver,
) {
    let top = dir_cfg.is_none();
    let meta = if top {
        std::fs::metadata(p)
    } else {
        std::fs::symlink_metadata(p)
    };
    let Ok(meta) = meta else { return };
    let wanted = if meta.is_dir() {
        let why = (!top).then(|| crate::skipped_dir(p)).flatten();
        if let Some(why) = why.filter(|_| WALK_VERBOSE.load(Ordering::Relaxed)) {
            eprintln!(
                "skipped dir {}: {why}",
                shown(p.strip_prefix(".").unwrap_or(p))
            );
        }
        why.is_none()
    } else {
        meta.is_file() && (top || crate::is_walked_file(p))
    };
    if !wanted {
        return;
    }
    // an argument also answers to the configs above its own, as in a walk from there
    if top && res.excluded(p).unwrap_or_else(|e| die(e)) {
        return;
    }
    let cfg = dir_cfg
        .cloned()
        .unwrap_or_else(|| res.of(p))
        .unwrap_or_else(|e| die(e));
    if cfg.excluded(p) {
        return;
    }
    if meta.is_dir() {
        let rd = match std::fs::read_dir(p) {
            Ok(rd) => rd,
            Err(e) => {
                eprintln!("groovyfmt: {}: {e}", shown(p));
                WALK_FAILED.store(true, Ordering::Relaxed);
                return;
            }
        };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        // without a config file of its own, `p` has the config that governs its parent
        let named = |e: &PathBuf| {
            e.file_name()
                .is_some_and(|n| config::FILE_NAMES.iter().any(|c| n == *c))
        };
        let inner = if entries.iter().any(named) {
            res.inside(p)
        } else {
            Ok(cfg)
        };
        for path in entries {
            walk(&path, Some(&inner), out, res);
        }
    } else {
        // `./src/A.groovy` reads as `src/A.groovy`, as in git and cargo output
        out.push((p.strip_prefix(".").unwrap_or(p).to_path_buf(), cfg));
    }
}

/// The files to process, named as the user typed them, and for each symlink to a file named
/// on the command line, the file it points to. That file is the one read and rewritten, so
/// the link stays a link; resolving it once here means a link retargeted later can't
/// redirect the write. Only the final component is checked again at write time (both write
/// paths refuse a symlink there); the directories above it are not pinned, which needs a
/// concurrent local attacker, as in the audit's accepted risk.
///
/// With `write`, an argument that is itself a symlink to somewhere outside the working
/// directory is refused (exit 2): `groovyfmt *` or `git ls-files` in an untrusted checkout
/// can hand us a committed link as an argument, but not a path beneath one. So only the
/// argument's own last component is checked; `../other-project`, absolute paths and paths
/// through a symlinked parent (`/tmp` on macOS) are the user's own choice.
pub fn collect(
    paths: &[PathBuf],
    res: &mut Resolver,
    write: bool,
) -> (Vec<PathBuf>, HashMap<PathBuf, PathBuf>, Configs) {
    let mut files = Vec::new();
    let mut links = HashMap::new();
    let cwd = std::fs::canonicalize(".").ok();
    for p in paths {
        if !p.exists() {
            die(format!("{}: no such file or directory", display_path(p)));
        }
        // components() drops a trailing `/` or `/.`, which would make the link resolve
        let last: PathBuf = p.components().collect();
        if write && last.is_symlink() {
            let real = std::fs::canonicalize(p)
                .unwrap_or_else(|e| die(format!("{}: {e}", display_path(p))));
            if cwd.as_ref().is_none_or(|c| !real.starts_with(c)) {
                eprintln!(
                    "groovyfmt: {}: symlink target is outside the working directory, not formatted",
                    shown(p)
                );
                WALK_FAILED.store(true, Ordering::Relaxed);
                continue;
            }
        }
        let n = files.len();
        walk(p, None, &mut files, res);
        if files.len() > n && p.is_symlink() && p.is_file() {
            match std::fs::canonicalize(p) {
                // excluding the target excludes it under any name
                Ok(real) if res.excluded(&real).unwrap_or_else(|e| die(e)) => files.truncate(n),
                Ok(real) => {
                    links.insert(files[n].0.clone(), real);
                }
                Err(e) => die(format!("{}: {e}", display_path(p))),
            }
        }
    }
    // overlapping arguments (`. src`, a file named twice or spelled differently): each file
    // once, first-seen order
    let mut seen = std::collections::HashSet::new();
    files.retain(|(f, _)| seen.insert(std::fs::canonicalize(f).unwrap_or_else(|_| f.clone())));
    let cfgs = files.iter().cloned().collect();
    (files.into_iter().map(|(f, _)| f).collect(), links, cfgs)
}

/// Run `f` over every file on all cores; results come back in input order. A panic while
/// handling one file becomes that file's error instead of aborting the whole run.
pub fn par_map<T: Send>(
    files: &[PathBuf],
    f: impl Fn(&Path) -> Result<T, String> + Sync,
) -> Vec<Result<T, String>> {
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(files.len().max(1));
    let mut out: Vec<Option<Result<T, String>>> = files.iter().map(|_| None).collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = files.get(i) else { break };
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(path)))
                            .unwrap_or_else(|_| {
                                Err(format!("{}: internal error, please report", shown(path)))
                            });
                        mine.push((i, r));
                    }
                    mine
                })
            })
            .collect();
        for h in handles {
            // a worker that died outside catch_unwind took its results with it
            for (i, r) in h.join().unwrap_or_default() {
                out[i] = Some(r);
            }
        }
    });
    out.into_iter()
        .zip(files)
        .map(|(r, path)| {
            r.unwrap_or_else(|| Err(format!("{}: internal error, please report", shown(path))))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn par_map_turns_a_panic_into_that_files_error() {
        let files: Vec<PathBuf> = (0..20)
            .map(|i| PathBuf::from(format!("F{i}.groovy")))
            .collect();
        let r = par_map(&files, |p| {
            if p == Path::new("F7.groovy") {
                panic!("boom");
            }
            Ok(p.display().to_string())
        });
        assert_eq!(r.len(), files.len());
        for (f, r) in files.iter().zip(&r) {
            if f == Path::new("F7.groovy") {
                assert_eq!(
                    r,
                    &Err("F7.groovy: internal error, please report".to_string())
                );
            } else {
                assert_eq!(r, &Ok(f.display().to_string()));
            }
        }
    }

    #[test]
    fn par_map_keeps_results_with_their_files_when_a_worker_dies() {
        // a panic payload that panics again when dropped kills its worker outside catch_unwind
        struct Bomb;
        impl Drop for Bomb {
            fn drop(&mut self) {
                panic!("dropped");
            }
        }
        let files: Vec<PathBuf> = (0..200)
            .map(|i| PathBuf::from(format!("F{i}.groovy")))
            .collect();
        let r = par_map(&files, |p| {
            if p == Path::new("F3.groovy") {
                std::panic::panic_any(Bomb);
            }
            Ok(p.display().to_string())
        });
        assert_eq!(r.len(), files.len());
        let lost = Err("F3.groovy: internal error, please report".to_string());
        assert_eq!(r[3], lost);
        for (f, r) in files.iter().zip(&r) {
            // the dead worker's other files are errors too; the rest keep their own results
            let mine = Ok(f.display().to_string());
            let error = Err(format!("{}: internal error, please report", f.display()));
            assert!(*r == mine || *r == error, "{f:?}: {r:?}");
        }
    }
}
