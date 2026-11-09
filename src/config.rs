//! `.groovyfmt.toml` / `groovyfmt.toml`, found by walking up from the working directory.
//!
//! ```toml
//! indent = 4                  # spaces per level, 1..=16
//! indent_after_labels = false # indent Spock when:/then: blocks under their label
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
                (other, _) => {
                    return Err(format!(
                        "unknown key `{other}` (expected `indent`, `indent_after_labels` or `[lint]`)"
                    ));
                }
            }
        }
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
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
