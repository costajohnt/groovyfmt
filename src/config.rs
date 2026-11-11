//! `.groovyfmt.toml` / `groovyfmt.toml`. Each file is governed by the nearest one found by
//! walking up from that file's own directory (for `--stdin-filepath`, from that path's
//! directory); a nested config replaces its parents' entirely, nothing is merged. A
//! directory is entered unless the config governing its parent excludes it. `--config`
//! replaces discovery for every file. See `Resolver`.
//!
//! ```toml
//! indent = 4                  # spaces per level, 1..=16
//! continuation_indent = 8     # arguments, literals, continued lines (default: indent)
//! use_tabs = false            # a tab per `indent` columns of leading indentation
//! indent_after_labels = false # indent Spock when:/then: blocks under their label
//! normalize_spacing = false   # also fix the spaces between tokens within a line
//! exclude = ["generated/**"]  # globs relative to this file's directory
//! baseline = ".groovyfmt-baseline.json" # lint reports only what this file doesn't hold
//!
//! [lint]                      # rule name = on/off; omitted rules use their default
//! UnnecessaryGString = true
//! UnusedImport = false
//! ```

use crate::format::Style;
use crate::lint::RULES;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const FILE_NAMES: [&str; 2] = [".groovyfmt.toml", "groovyfmt.toml"];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub style: Style,
    /// Explicit rule settings from the file; rules not listed use `lint::RULES` defaults.
    pub rules: BTreeMap<String, bool>,
    /// Glob patterns of files and directories to skip, relative to `root`.
    pub exclude: Vec<String>,
    /// The config file's directory, absolute; set by `load`.
    pub root: PathBuf,
    /// The file this was loaded from, as given to `load`; `None` for the defaults.
    pub file: Option<PathBuf>,
    /// The lint baseline, relative to `root` until `load` makes it absolute.
    pub baseline: Option<PathBuf>,
}

impl Config {
    pub fn rule_enabled(&self, rule: &str) -> bool {
        self.rules.get(rule).copied().unwrap_or_else(|| {
            RULES
                .iter()
                .find(|r| r.name == rule)
                .is_some_and(|r| r.default)
        })
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| e.message().to_string())?;
        let mut cfg = Config::default();
        for (key, value) in &table {
            match (key.as_str(), value) {
                ("indent", toml::Value::Integer(n)) if (1..=16).contains(n) => {
                    cfg.style.indent = *n as usize
                }
                ("indent", _) => return Err("`indent` must be an integer from 1 to 16".into()),
                ("continuation_indent", toml::Value::Integer(n)) if (1..=16).contains(n) => {
                    cfg.style.continuation = Some(*n as usize)
                }
                ("continuation_indent", _) => {
                    return Err("`continuation_indent` must be an integer from 1 to 16".into());
                }
                ("indent_after_labels", toml::Value::Boolean(b)) => {
                    cfg.style.indent_after_labels = *b
                }
                ("indent_after_labels", _) => {
                    return Err("`indent_after_labels` must be true or false".into());
                }
                ("use_tabs", toml::Value::Boolean(b)) => cfg.style.use_tabs = *b,
                ("use_tabs", _) => return Err("`use_tabs` must be true or false".into()),
                ("normalize_spacing", toml::Value::Boolean(b)) => cfg.style.normalize_spacing = *b,
                ("normalize_spacing", _) => {
                    return Err("`normalize_spacing` must be true or false".into());
                }
                ("lint", toml::Value::Table(rules)) => {
                    for (rule, on) in rules {
                        if !RULES.iter().any(|r| r.name == rule) {
                            let known: Vec<_> = RULES.iter().map(|r| r.name).collect();
                            return Err(format!(
                                "unknown lint rule `{rule}` (known: {})",
                                known.join(", ")
                            ));
                        }
                        let Some(on) = on.as_bool() else {
                            return Err(format!("`lint.{rule}` must be true or false"));
                        };
                        cfg.rules.insert(rule.clone(), on);
                    }
                }
                ("lint", _) => return Err("`lint` must be a table of rule = true/false".into()),
                ("exclude", toml::Value::Array(pats)) if pats.iter().all(|p| p.is_str()) => {
                    cfg.exclude = pats
                        .iter()
                        .filter_map(|p| p.as_str().map(String::from))
                        .collect()
                }
                ("exclude", _) => {
                    return Err("`exclude` must be a list of glob strings".into());
                }
                // a pull request can set this, so it may only name a file in the config's own
                // directory tree: relative, and no `..`
                ("baseline", toml::Value::String(f))
                    if !f.is_empty()
                        && Path::new(f).components().all(|c| {
                            matches!(
                                c,
                                std::path::Component::Normal(_) | std::path::Component::CurDir
                            )
                        }) =>
                {
                    cfg.baseline = Some(PathBuf::from(f))
                }
                ("baseline", _) => {
                    return Err(
                        "`baseline` must be a relative path inside the config's directory, without `..`"
                            .into(),
                    );
                }
                (other, _) => {
                    return Err(format!(
                        "unknown key `{other}` (expected `indent`, `continuation_indent`, `use_tabs`, `indent_after_labels`, `normalize_spacing`, `exclude`, `baseline` or `[lint]`)"
                    ));
                }
            }
        }
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut cfg = Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let abs = normalize(&std::env::current_dir().unwrap_or_default().join(path));
        cfg.root = abs.parent().map(Path::to_path_buf).unwrap_or_default();
        cfg.file = Some(path.to_path_buf());
        cfg.baseline = cfg.baseline.map(|b| cfg.root.join(b));
        Ok(cfg)
    }

    /// Whether `path` (relative to the working directory, or absolute), or any directory
    /// between it and `root`, matches an `exclude` pattern. A path outside `root` never does.
    pub fn excluded(&self, path: &Path) -> bool {
        if self.exclude.is_empty() {
            return false;
        }
        let abs = normalize(&std::env::current_dir().unwrap_or_default().join(path));
        let Ok(rel) = abs.strip_prefix(&self.root) else {
            return false;
        };
        let segs: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
        self.exclude.iter().any(|pat| {
            let pat: Vec<&str> = pat
                .split('/')
                .filter(|s| !s.is_empty() && *s != ".")
                .collect();
            (1..=segs.len()).any(|n| glob(&pat, &segs[..n]))
        })
    }
}

/// `a/./b/../c` as `a/c`, without touching the file system (the path may not exist).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// Path segments against pattern segments: `**` matches any number of segments (including
/// none); within a segment `*` matches any run of characters and `?` one character.
/// The config can come from an untrusted pull request, so this must stay polynomial:
/// O(pattern × path) segment comparisons, however many `**` the pattern stacks.
fn glob(pat: &[&str], path: &[&str]) -> bool {
    wildcard(pat, path, |p| *p == "**", |p, s| segment(p, s))
}

/// `*` and `?` within one path segment.
fn segment(pat: &str, s: &str) -> bool {
    let (p, s): (Vec<char>, Vec<char>) = (pat.chars().collect(), s.chars().collect());
    wildcard(&p, &s, |c| *c == '*', |c, x| *c == '?' || c == x)
}

/// Wildcard match where `star` elements match any run of items and every other element
/// matches exactly one item (`one`). Greedy, backtracking only to the most recent star,
/// which is enough because a later star can absorb anything an earlier one could.
fn wildcard<P, S>(
    p: &[P],
    s: &[S],
    star: impl Fn(&P) -> bool,
    one: impl Fn(&P, &S) -> bool,
) -> bool {
    let (mut i, mut j, mut back, mut mark) = (0, 0, None, 0);
    while j < s.len() {
        if i < p.len() && star(&p[i]) {
            back = Some(i);
            mark = j;
            i += 1;
        } else if i < p.len() && one(&p[i], &s[j]) {
            i += 1;
            j += 1;
        } else if let Some(b) = back {
            i = b + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(star)
}

/// The config governing each path, found once per directory: `fixed` for every path if
/// set, else the nearest config file at or above the path's directory, else the defaults.
/// `indent` (`--indent`) overrides whichever applies. Paths are resolved lexically against
/// the working directory, as typed, like `--stdin-filepath`.
pub struct Resolver {
    cwd: PathBuf,
    indent: Option<usize>,
    fixed: Option<Arc<Config>>,
    defaults: Arc<Config>,
    /// The nearest config file at or above each directory looked at.
    found: HashMap<PathBuf, Option<PathBuf>>,
    /// Each config file read, by path.
    loaded: HashMap<PathBuf, Arc<Config>>,
}

impl Resolver {
    pub fn new(fixed: Option<Config>, indent: Option<usize>) -> Resolver {
        let mut r = Resolver {
            cwd: std::env::current_dir().unwrap_or_default(),
            indent,
            fixed: None,
            defaults: Arc::default(),
            found: HashMap::new(),
            loaded: HashMap::new(),
        };
        r.fixed = fixed.map(|c| r.finish(c));
        r.defaults = r.finish(Config::default());
        r
    }

    /// Whether `path` (named on the command line, or by `--stdin-filepath`) is excluded:
    /// by its own config, or by a config above it that would have pruned it in a walk from
    /// there. Each config decides down to the directory holding the next config below it,
    /// and the highest is asked first, so a config below an excluded directory is not read.
    pub fn excluded(&mut self, path: &Path) -> Result<bool, String> {
        if let Some(c) = &self.fixed {
            return Ok(c.excluded(path));
        }
        let abs = normalize(&self.cwd.join(path));
        // (config file, the deepest path it decides), nearest first
        let mut chain = Vec::new();
        let (mut dir, mut target) = (abs.parent().map(Path::to_path_buf), abs.clone());
        while let Some(file) = dir.and_then(|d| self.find(&d)) {
            let root = file.parent().map(Path::to_path_buf).unwrap_or_default();
            dir = root.parent().map(Path::to_path_buf);
            chain.push((file, std::mem::replace(&mut target, root)));
        }
        for (file, target) in chain.iter().rev() {
            if self.load(file)?.excluded(target) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The config for `path`'s directory: for a file, the one that formats and lints it; for
    /// a directory, the one whose `exclude` decides whether it is entered (a directory's own
    /// config can't exclude it).
    pub fn of(&mut self, path: &Path) -> Result<Arc<Config>, String> {
        let abs = normalize(&self.cwd.join(path));
        self.dir(abs.parent().unwrap_or(&abs))
    }

    /// The config governing files directly in `dir`, an absolute normalized path. A config
    /// file that fails to load is an error only when something under it asks.
    pub fn dir(&mut self, dir: &Path) -> Result<Arc<Config>, String> {
        if let Some(c) = &self.fixed {
            return Ok(c.clone());
        }
        match self.find(dir) {
            Some(file) => self.load(&file),
            None => Ok(self.defaults.clone()),
        }
    }

    /// The nearest config file in `dir`, an absolute normalized path, or above it.
    fn find(&mut self, dir: &Path) -> Option<PathBuf> {
        if let Some(found) = self.found.get(dir) {
            return found.clone();
        }
        let found = FILE_NAMES
            .iter()
            .map(|n| dir.join(n))
            .find(|p| p.is_file())
            .or_else(|| dir.parent().and_then(|up| self.find(up)));
        self.found.insert(dir.to_path_buf(), found.clone());
        found
    }

    fn load(&mut self, file: &Path) -> Result<Arc<Config>, String> {
        if let Some(c) = self.loaded.get(file) {
            return Ok(c.clone());
        }
        let cfg = self.finish(Config::load(file)?);
        self.loaded.insert(file.to_path_buf(), cfg.clone());
        Ok(cfg)
    }

    /// The config governing files directly in `dir`, relative to the working directory or
    /// absolute.
    pub fn inside(&mut self, dir: &Path) -> Result<Arc<Config>, String> {
        let abs = normalize(&self.cwd.join(dir));
        self.dir(&abs)
    }

    fn finish(&self, mut cfg: Config) -> Arc<Config> {
        if let Some(n) = self.indent {
            cfg.style.indent = n;
        }
        Arc::new(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let c =
            Config::parse("indent = 2\n[lint]\nUnusedImport = false\nUnnecessaryGString = true\n")
                .unwrap();
        assert_eq!(c.style.indent, 2);
        assert!(!c.style.indent_after_labels);
        assert!(
            Config::parse("indent_after_labels = true")
                .unwrap()
                .style
                .indent_after_labels
        );
        assert!(Config::parse("indent_after_labels = 1").is_err());
        assert_eq!(
            Config::parse("continuation_indent = 8")
                .unwrap()
                .style
                .continuation,
            Some(8)
        );
        assert!(Config::parse("continuation_indent = 0").is_err());
        assert!(!Config::default().style.use_tabs);
        assert!(Config::parse("use_tabs = true").unwrap().style.use_tabs);
        assert!(
            Config::parse("use_tabs = 1")
                .unwrap_err()
                .contains("`use_tabs` must be true or false")
        );
        assert!(!Config::default().style.normalize_spacing);
        assert!(
            Config::parse("normalize_spacing = true")
                .unwrap()
                .style
                .normalize_spacing
        );
        assert!(
            Config::parse("normalize_spacing = \"yes\"")
                .unwrap_err()
                .contains("`normalize_spacing` must be true or false")
        );
        assert!(!c.rule_enabled("UnusedImport"));
        assert!(c.rule_enabled("UnnecessaryGString"));
        assert!(
            c.rule_enabled("EmptyCatchBlock"),
            "unlisted rule keeps its default"
        );
        assert!(
            !Config::default().rule_enabled("UnnecessaryGString"),
            "off by default"
        );
    }

    #[test]
    fn baseline_paths_stay_inside_the_config_directory() {
        for ok in ["b.json", "./ci/b.json", "a/b/c.json"] {
            let cfg = Config::parse(&format!("baseline = \"{ok}\"")).unwrap();
            assert_eq!(cfg.baseline, Some(PathBuf::from(ok)));
        }
        for bad in [
            "..",
            "../b.json",
            "a/../../b.json",
            "a/../b.json",
            "/etc/passwd",
            "../../.github/workflows/x.yml",
            "",
        ] {
            let e = Config::parse(&format!("baseline = \"{bad}\"")).unwrap_err();
            assert!(e.contains("relative path inside"), "{bad}: {e}");
        }
        assert!(Config::parse("baseline = 1").is_err());
        // loaded: resolved against the config's own directory
        let dir = std::env::temp_dir().join(format!("groovyfmt-bl-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/.groovyfmt.toml"), "baseline = \"b.json\"\n").unwrap();
        let cfg = Config::load(&dir.join("sub/.groovyfmt.toml")).unwrap();
        assert_eq!(cfg.baseline, Some(cfg.root.join("b.json")));
        assert!(cfg.root.ends_with("sub"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_typos_and_bad_values() {
        assert!(
            Config::parse("indnet = 2")
                .unwrap_err()
                .contains("unknown key `indnet`")
        );
        assert!(Config::parse("indent = 0").is_err());
        assert!(Config::parse("indent = \"4\"").is_err());
        assert!(
            Config::parse("[lint]\nUnusedImports = false")
                .unwrap_err()
                .contains("unknown lint rule")
        );
        assert!(Config::parse("[lint]\nUnusedImport = 1").is_err());
        assert!(Config::parse("indent = ").is_err(), "TOML syntax error");
        for bad in [
            "exclude = \"a\"",
            "exclude = [1]",
            "exclude = [\"a\", true]",
        ] {
            assert!(
                Config::parse(bad)
                    .unwrap_err()
                    .contains("`exclude` must be a list of glob strings"),
                "{bad}"
            );
        }
        assert!(Config::parse("exclude = []").unwrap().exclude.is_empty());
    }

    #[test]
    fn glob_segments() {
        let m = |pat: &str, path: &str| {
            let pat: Vec<&str> = pat.split('/').collect();
            let path: Vec<&str> = path.split('/').collect();
            glob(&pat, &path)
        };
        assert!(m("*.groovy", "A.groovy"));
        assert!(!m("*.groovy", "src/A.groovy"), "* stays in one segment");
        assert!(m("src/*.groovy", "src/A.groovy"));
        assert!(m("?.groovy", "A.groovy") && !m("?.groovy", "AB.groovy"));
        assert!(m("**/*.groovy", "A.groovy"), "** matches zero segments");
        assert!(m("**/*.groovy", "a/b/c/A.groovy"));
        assert!(m("a/**/z", "a/z") && m("a/**/z", "a/b/c/z") && !m("a/**/z", "a/b/c"));
        assert!(m("**/gen", "x/y/gen") && !m("**/gen", "x/gen/y"));
        assert!(m("a*b*c", "abc") && m("a*b*c", "aXbYbZc") && !m("a*b*c", "acb"));
        assert!(m("*", "é") && m("?", "é"), "characters, not bytes");
        assert!(
            !m("vendor", "src/vendor"),
            "anchored at the config directory"
        );
    }

    #[test]
    fn stacked_double_stars_stay_polynomial() {
        // exponential backtracking took minutes on 22 stars; a config from a fork PR could
        // hang CI. Twice the stars may cost at most 8 times as much (it is quadratic: 4x).
        let run = |stars: usize| {
            let pat: Vec<&str> = std::iter::repeat_n(["**", "a"], stars)
                .flatten()
                .chain(["zz"])
                .collect();
            let path = vec!["a"; 2 * stars];
            assert!(!glob(&pat, &path));
            let cfg = Config {
                exclude: vec![pat.join("/")],
                root: PathBuf::from("/p"),
                ..Config::default()
            };
            assert!(!cfg.excluded(&Path::new("/p").join(path.join("/"))));
        };
        crate::scaling::assert_scales("stacked **", run, 11, 2, 8);
        assert!(
            segment("*b", "*xb"),
            "a literal * in a name doesn't stop a star"
        );
    }

    #[test]
    fn excluded_relative_to_root() {
        let cfg = Config {
            exclude: vec![
                "gen/**".into(),
                "**/*.gradle".into(),
                "./one/A.groovy".into(),
            ],
            root: PathBuf::from("/p"),
            ..Config::default()
        };
        assert!(cfg.excluded(Path::new("/p/gen")));
        assert!(cfg.excluded(Path::new("/p/gen/x/A.groovy")));
        assert!(
            cfg.excluded(Path::new("/p/sub/../gen/A.groovy")),
            "normalized"
        );
        assert!(cfg.excluded(Path::new("/p/a/b/build.gradle")));
        assert!(cfg.excluded(Path::new("/p/one/A.groovy")));
        assert!(!cfg.excluded(Path::new("/p/src/gen/A.groovy")));
        assert!(!cfg.excluded(Path::new("/p")), "the root itself");
        assert!(!cfg.excluded(Path::new("/q/gen/A.groovy")), "outside root");
        let none = Config {
            root: PathBuf::from("/p"),
            ..Config::default()
        };
        assert!(!none.excluded(Path::new("/p/gen")));
    }

    #[test]
    fn resolver_finds_the_nearest_config_once_per_directory() {
        let root = std::env::temp_dir().join(format!("groovyfmt-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join(".groovyfmt.toml"), "indent = 2\n").unwrap();
        std::fs::write(root.join("sub/groovyfmt.toml"), "indent = 3\n").unwrap();
        let mut r = Resolver::new(None, None);
        let top = r.of(&root.join("a/b/A.groovy")).unwrap();
        assert_eq!(top.style.indent, 2, "found in a parent");
        assert_eq!(top.file, Some(root.join(".groovyfmt.toml")));
        assert!(
            Arc::ptr_eq(&top, &r.of(&root.join("T.groovy")).unwrap()),
            "shared"
        );
        let sub = r.of(&root.join("sub/deep/S.groovy")).unwrap();
        assert_eq!(sub.style.indent, 3, "nested replaces, nearest wins");
        assert_eq!(
            r.of(&root.join("sub")).unwrap().style.indent,
            2,
            "a dir: its parent's"
        );
        assert_eq!(r.inside(&root.join("sub")).unwrap().style.indent, 3);
        // memoized: removing the files changes nothing for directories already seen
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(r.of(&root.join("a/b/C.groovy")).unwrap().style.indent, 2);
        assert_eq!(
            r.of(&root.join("sub/deep/D.groovy")).unwrap().style.indent,
            3
        );

        // excludes from the configs above a path, highest first
        let root = root.with_extension("excl");
        let _ = std::fs::remove_dir_all(&root);
        for (dir, toml) in [
            ("", "exclude = [\"vendor\", \"**/gen\"]\n"),
            ("vendor/x", "indent = \n"),
            ("sub", "exclude = [\"skip\"]\n"),
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join(".groovyfmt.toml"), toml).unwrap();
        }
        let mut r = Resolver::new(None, None);
        let mut ex = |p: &str| r.excluded(&root.join(p));
        assert_eq!(
            ex("vendor/x/A.groovy"),
            Ok(true),
            "broken config never read"
        );
        assert_eq!(ex("gen/A.groovy"), Ok(true));
        assert_eq!(ex("sub/gen/A.groovy"), Ok(false), "replaced inside sub");
        assert_eq!(ex("sub/skip/A.groovy"), Ok(true));
        assert_eq!(ex("sub/A.groovy"), Ok(false));
        assert_eq!(ex("skip/A.groovy"), Ok(false), "sub's patterns stay in sub");
        assert!(r.of(&root.join("vendor/x/A.groovy")).is_err());
        std::fs::remove_dir_all(&root).unwrap();

        let mut fixed = Resolver::new(Some(Config::default()), Some(5));
        assert_eq!(
            fixed.of(&root.join("sub/X.groovy")).unwrap().style.indent,
            5
        );
        assert_eq!(fixed.of(&root.join("sub/X.groovy")).unwrap().file, None);
    }
}
