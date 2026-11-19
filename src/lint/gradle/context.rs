//! What a build script's rules read beyond the script: the Gradle version its wrapper pins,
//! the `.gradle` scripts around it that may configure it, and its `gradle.properties`.
//! Through `Projects` (a CLI run, the LSP), each wrapper file and script is read and
//! parsed once while it stays the same on disk, so a multi-module build's root script is
//! parsed once and not once per subproject file (M15). Each use checks the files again (a
//! directory listing and a `stat` per file), so a change made by any tool is seen.
//! Without `Projects` (`lint_file`), they are read once per linted file.

use super::super::jenkins::{Call, call};
use super::{apply_argument, convention, declares, may_apply_java};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// The most bytes read of a script that may configure the one linted (`Build::declared`).
const MAX_SCRIPT_BYTES: u64 = 4 * 1024 * 1024;

/// The real file is a few hundred bytes.
pub(super) const MAX_WRAPPER_BYTES: u64 = 64 * 1024;

/// What a file was when it was read: a regular file, its length and its modification time.
/// ponytail: a rewrite to the same length within one tick of the filesystem's clock (ns on
/// APFS, a few ms on some Linux filesystems) goes unseen until the next change or `clear`.
type Stamp = (bool, u64, Option<SystemTime>);

/// A wrapper's Gradle version, (major, minor).
type Version = Option<(u32, u32)>;

fn stamp(p: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.is_file(), m.len(), m.modified().ok()))
}

/// Each wrapper file's version and each `.gradle` script's facts, by path, with the file's
/// `Stamp` when read; an entry whose file changed since is read again.
#[derive(Default)]
pub(crate) struct Cache {
    wrappers: Mutex<HashMap<PathBuf, (Stamp, Version)>>,
    scripts: Mutex<HashMap<PathBuf, Arc<Entry>>>,
}

/// One script as read at `stamp`; `None` when it can't be read.
struct Entry {
    stamp: Stamp,
    script: OnceLock<Option<Arc<Script>>>,
}

impl Cache {
    pub(crate) fn clear(&self) {
        self.wrappers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.scripts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// The facts of the script at `p` in `dir`, read again when it changed.
    fn script(&self, p: &Path, dir: &Path) -> Option<Arc<Script>> {
        let stamp = stamp(p)?;
        let entry = {
            let mut map = self.scripts.lock().unwrap_or_else(|e| e.into_inner());
            match map.get(p) {
                Some(e) if e.stamp == stamp => e.clone(),
                _ => {
                    let e = Arc::new(Entry {
                        stamp,
                        script: OnceLock::new(),
                    });
                    map.insert(p.to_path_buf(), e.clone());
                    e
                }
            }
        };
        // a change between the stat and the read leaves a stamp that the next stat misses
        entry
            .script
            .get_or_init(|| Script::read(p, dir).map(Arc::new))
            .clone()
    }

    /// The version in the wrapper file `props`, read again when it changed.
    fn wrapper(&self, props: &Path) -> Option<(u32, u32)> {
        let stamp = stamp(props)?;
        let mut map = self.wrappers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((s, v)) = map.get(props)
            && *s == stamp
        {
            return *v;
        }
        let v = read_version(props);
        map.insert(props.to_path_buf(), (stamp, v));
        v
    }
}

/// What a nearby `.gradle` script tells the one linted.
struct Script {
    /// the names it makes extra properties of
    ext: HashSet<String>,
    /// whether it may apply a Java plugin (`may_apply_java`)
    java: bool,
    /// the configurations it makes anywhere, for a script it applies with `apply from:`
    declared: HashSet<String>,
    /// the ones it makes in `allprojects { }` or `subprojects { }`, for the others
    for_projects: HashSet<String>,
    /// the scripts it applies with `apply from:`, relative to its directory as written
    applies: Vec<PathBuf>,
}

impl Script {
    fn read(path: &Path, dir: &Path) -> Option<Script> {
        let src = read_small(path, MAX_SCRIPT_BYTES)?;
        let src = src.as_str();
        let tree = crate::parse(src);
        let root = tree.root_node();
        let (mut java, mut declared, mut for_projects, mut applies) =
            (false, HashSet::new(), HashSet::new(), Vec::new());
        let mut names = HashSet::new();
        crate::preorder(root, |n| {
            convention(src, n, &mut Vec::new(), &mut names);
            java |= may_apply_java(src, n);
            applies.extend(apply_argument(src, n, "from").map(|t| dir.join(t)));
            declares(src, n, &mut declared);
            true
        });
        crate::preorder(root, |n| match call(n, src) {
            Some(Call {
                name: "allprojects" | "subprojects",
                body: Some(b),
                ..
            }) => {
                crate::preorder(b, |x| {
                    declares(src, x, &mut for_projects);
                    true
                });
                false
            }
            _ => true,
        });
        Some(Script {
            ext: names.into_iter().map(str::to_string).collect(),
            java,
            declared,
            for_projects,
            applies,
        })
    }
}

/// The build around one script: its path, and what of the build each rule asked for so far.
pub(super) struct Build<'a> {
    path: &'a Path,
    cache: Option<&'a Cache>,
    wrapper: OnceCell<Option<(u32, u32)>>,
    nearby: OnceCell<Option<Nearby>>,
    declared: OnceCell<HashSet<String>>,
    ext: OnceCell<HashSet<String>>,
}

/// The other `.gradle` scripts in the directory of the canonical `me` and the ones above
/// it, up to the build's root (the nearest directory with `settings.gradle`,
/// `settings.gradle.kts` or `.git`; without one, only the script's own directory).
struct Nearby {
    me: PathBuf,
    scripts: Vec<Arc<Script>>,
}

impl<'a> Build<'a> {
    pub(super) fn new(path: &'a Path, cache: Option<&'a Cache>) -> Build<'a> {
        Build {
            path,
            cache,
            wrapper: OnceCell::new(),
            nearby: OnceCell::new(),
            declared: OnceCell::new(),
            ext: OnceCell::new(),
        }
    }

    pub(super) fn path(&self) -> &Path {
        self.path
    }

    /// The major Gradle version of the nearest `gradle/wrapper/gradle-wrapper.properties`
    /// in the script's directory or above it (`distributionUrl=.../gradle-6.9-bin.zip` is
    /// 6). A build pinned below 7.0 runs a Gradle that still has the removed
    /// configurations. Like the search for `.groovyfmt.toml`, the walk goes up to the
    /// filesystem root, so a wrapper in a directory above the project counts when the
    /// project has none. A path that doesn't exist (an unsaved editor buffer) is walked as
    /// written.
    pub(super) fn wrapper_major(&self) -> Option<u32> {
        self.wrapper_version().map(|v| v.0)
    }

    /// The nearest wrapper's Gradle version as (major, minor), as for `wrapper_major`.
    pub(super) fn wrapper_version(&self) -> Option<(u32, u32)> {
        *self.wrapper.get_or_init(|| {
            let dir = self
                .path
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            let props = dir
                .ancestors()
                .map(|d| d.join("gradle/wrapper/gradle-wrapper.properties"))
                .find(|p| p.is_file())?;
            match self.cache {
                Some(c) => c.wrapper(&props),
                None => read_version(&props),
            }
        })
    }

    fn nearby(&self) -> Option<&Nearby> {
        self.nearby
            .get_or_init(|| {
                let me = self.path.canonicalize().ok()?;
                let root = me.ancestors().skip(1).find(|d| {
                    ["settings.gradle", "settings.gradle.kts", ".git"]
                        .iter()
                        .any(|f| d.join(f).exists())
                });
                let mut scripts = Vec::new();
                for dir in me.ancestors().skip(1) {
                    for p in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                        let p = p.path();
                        if p.extension().is_none_or(|e| e != "gradle") || p == me {
                            continue;
                        }
                        scripts.extend(match self.cache {
                            Some(c) => c.script(&p, dir),
                            None => Script::read(&p, dir).map(Arc::new),
                        });
                    }
                    if root.is_none_or(|r| r == dir) {
                        break;
                    }
                }
                Some(Nearby { me, scripts })
            })
            .as_ref()
    }

    /// The configurations that the nearby scripts make for this one: all those of a
    /// script that applies it with `apply from: 'deps.gradle'`, as a build script declares
    /// `testCompile` and the `dependencies.gradle` it applies uses it (#217), and those any
    /// of them makes in `allprojects { }` or `subprojects { }`, as a root `build.gradle`
    /// declares `compile` for every project and a subproject uses it (#241).
    pub(super) fn declared(&self) -> &HashSet<String> {
        self.declared.get_or_init(|| {
            let mut declared = HashSet::new();
            if let Some(n) = self.nearby() {
                for s in &n.scripts {
                    // the `apply from:` target must be a plain path to this script relative
                    // to the applying one's directory: a URL, an interpolated name or a
                    // missing file applies nothing here
                    if s.applies
                        .iter()
                        .any(|t| t.canonicalize().is_ok_and(|t| t == n.me))
                    {
                        declared.extend(s.declared.iter().cloned());
                    } else {
                        declared.extend(s.for_projects.iter().cloned());
                    }
                }
            }
            declared
        })
    }

    /// Whether one of the nearby scripts may apply a Java plugin: it applies one from
    /// `JVM_PLUGINS` (a subproject's `compile` comes from its parent's
    /// `subprojects { apply plugin: 'java' }`) or applies a script (`apply from:`), which
    /// may. Where none does, a plugin from elsewhere made the configuration (gradle-hdvl's
    /// SystemVerilog plugin makes `compile` and `testCompile`, #230). A script that isn't on
    /// disk (`--stdin` without a real `--stdin-filepath`) can't be told, so it counts as one
    /// that does.
    pub(super) fn jvm_plugin_nearby(&self) -> bool {
        self.nearby()
            .is_none_or(|n| n.scripts.iter().any(|s| s.java))
    }

    /// The names the nearby scripts (the ones in the directories above, which may configure
    /// this one with `allprojects { }`) make extra properties of.
    pub(super) fn ext_names(&self) -> &HashSet<String> {
        self.ext.get_or_init(|| {
            let mut names = HashSet::new();
            if let Some(n) = self.nearby() {
                for s in &n.scripts {
                    names.extend(s.ext.iter().cloned());
                }
            }
            names
        })
    }

    /// Whether a `gradle.properties` in the script's directory, or one above it up to the
    /// wrapper's, sets `name`: then it is a project property, which a build can still set
    /// (checked with Gradle 9.9.1).
    pub(super) fn gradle_property(&self, name: &str) -> bool {
        let dir = self
            .path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        for d in dir.ancestors() {
            let sets =
                read_small(&d.join("gradle.properties"), MAX_WRAPPER_BYTES).is_some_and(|p| {
                    p.lines().any(|l| {
                        l.trim_start()
                            .strip_prefix(name)
                            .is_some_and(|r| r.is_empty() || r.starts_with([' ', '\t', '=', ':']))
                    })
                });
            if sets {
                return true;
            }
            if d.join("gradle/wrapper/gradle-wrapper.properties").is_file() {
                return false;
            }
        }
        false
    }
}

/// The version the wrapper file `props` pins.
fn read_version(props: &Path) -> Option<(u32, u32)> {
    version(&read_small(props, MAX_WRAPPER_BYTES)?)
}

/// `p` if it is a regular file of at most `max` bytes, in UTF-8. A FIFO or device
/// is never opened (opening a FIFO waits for a writer); one swapped in between the check and
/// the open is the race the source reads accept too (see `write_in_place`).
fn read_small(p: &Path, max: u64) -> Option<String> {
    use std::io::Read;
    let ok = |m: std::fs::Metadata| m.is_file() && m.len() <= max;
    if !std::fs::metadata(p).is_ok_and(ok) {
        return None;
    }
    let f = std::fs::File::open(p).ok()?;
    if !f.metadata().is_ok_and(ok) {
        return None;
    }
    let mut bytes = Vec::new();
    f.take(max + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// `gradle-8.12.1-bin.zip` is (8, 12), `gradle-7.0-rc-1-bin.zip` (7, 0), `gradle-9-bin.zip` (9, 0).
fn version(props: &str) -> Option<(u32, u32)> {
    let url = props
        .lines()
        .find_map(|l| l.trim().strip_prefix("distributionUrl"))?;
    let mut parts = url.rsplit("gradle-").next()?.split(['.', '-']);
    let major = parts.next()?.parse().ok()?;
    Some((
        major,
        parts.next().and_then(|m| m.parse().ok()).unwrap_or(0),
    ))
}
