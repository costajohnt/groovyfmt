//! `.groovyfmt.toml` / `groovyfmt.toml`, found by walking up from the working directory
//! (or from the directory of `--stdin-filepath`).
//!
//! ```toml
//! indent = 4                  # spaces per level, 1..=16
//! continuation_indent = 8     # arguments, literals, continued lines (default: indent)
//! indent_after_labels = false # indent Spock when:/then: blocks under their label
//! exclude = ["generated/**"]  # globs relative to this file's directory
//!
//! [lint]                      # rule name = on/off; omitted rules use their default
//! UnnecessaryGString = true
//! UnusedImport = false
//! ```

use crate::format::Style;
use crate::lint::RULES;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
                (other, _) => {
                    return Err(format!(
                        "unknown key `{other}` (expected `indent`, `continuation_indent`, `indent_after_labels`, `exclude` or `[lint]`)"
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

/// Nearest config file in `start` or any parent directory.
pub fn discover(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .flat_map(|dir| FILE_NAMES.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
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
        // exponential backtracking took minutes on this; a config from a fork PR could hang CI
        let pat: Vec<&str> = std::iter::repeat_n(["**", "a"], 22)
            .flatten()
            .chain(["zz"])
            .collect();
        let path = vec!["a"; 44];
        let t = std::time::Instant::now();
        assert!(!glob(&pat, &path));
        let cfg = Config {
            exclude: vec![pat.join("/")],
            root: PathBuf::from("/p"),
            ..Config::default()
        };
        assert!(!cfg.excluded(&Path::new("/p").join(path.join("/"))));
        assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
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
    fn discovers_in_parent_dirs() {
        let root = std::env::temp_dir().join(format!("groovyfmt-cfg-{}", std::process::id()));
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join(".groovyfmt.toml"), "indent = 2\n").unwrap();
        assert_eq!(discover(&nested), Some(root.join(".groovyfmt.toml")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
