//! The lint baseline: the violations a codebase had when it adopted the linter, recorded by
//! `--lint --write-baseline` and left out of later runs, so only new ones are reported.
//!
//! A violation is identified by the file's path (relative to the baseline file's directory,
//! `/`-separated), its rule, a fingerprint of the text of the line it starts on (whitespace
//! runs collapsed, then hashed), and its index among the violations with the same path, rule
//! and fingerprint, in line order. Never by line number: edits elsewhere in the file don't
//! disturb it. Paths are only ever compared as strings; nothing is read or written through
//! them.
//!
//! ```json
//! {"version":1,"entries":[
//! {"path":"src/A.groovy","rule":"UnusedImport","fingerprint":"9d6bd2a6f4c1e0b2","index":0}
//! ]}
//! ```

use crate::lint::Violation;
use crate::report::string;
use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path};

/// Bumped on any change to the file format; another version is an error.
pub const VERSION: u32 = 1;

/// Where `--write-baseline` writes when nothing else names a file.
pub const DEFAULT_FILE: &str = ".groovyfmt-baseline.json";

/// (path, rule, fingerprint).
type Key = (String, String, String);

#[derive(Debug, Default, PartialEq)]
pub struct Baseline {
    /// How many violations each key has in the baseline.
    counts: HashMap<Key, usize>,
}

/// FNV-1a, 64 bits: stable across runs, platforms and Rust versions, unlike std's hasher.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The fingerprint of a line: its text with every whitespace run made one space and the
/// ends trimmed (so re-indenting, CR LF and trailing spaces don't change it), hashed.
pub fn fingerprint(line: &str) -> String {
    let collapsed: Vec<&str> = line.split_whitespace().collect();
    format!("{:016x}", fnv1a(&collapsed.join(" ")))
}

/// Each violation's (rule, fingerprint, index), in the order given.
pub fn keys(src: &str, vs: &[Violation]) -> Vec<(String, String, usize)> {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut order: Vec<usize> = (0..vs.len()).collect();
    order.sort_by_key(|&i| (vs[i].line, vs[i].col));
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut out = vec![(String::new(), String::new(), 0); vs.len()];
    for i in order {
        let fp = fingerprint(lines.get(vs[i].line - 1).copied().unwrap_or(""));
        let n = seen.entry((vs[i].rule.to_string(), fp.clone())).or_default();
        out[i] = (vs[i].rule.to_string(), fp, *n);
        *n += 1;
    }
    out
}

/// `path` relative to `dir`, both made absolute against the working directory and
/// normalized lexically, with `/` separators; `..` where `path` is outside `dir`.
pub fn relative(path: &Path, dir: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let (p, d) = (
        crate::config::normalize(&cwd.join(path)),
        crate::config::normalize(&cwd.join(dir)),
    );
    let (pc, dc): (Vec<Component>, Vec<Component>) =
        (p.components().collect(), d.components().collect());
    let common = pc.iter().zip(&dc).take_while(|(a, b)| a == b).count();
    let mut out: Vec<String> = vec!["..".to_string(); dc.len() - common];
    out.extend(
        pc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    out.join("/")
}

impl Baseline {
    pub fn parse(text: &str) -> Result<Baseline, String> {
        let doc = crate::json::parse(text)?;
        match doc.get("version").u32() {
            Some(VERSION) => {}
            Some(v) => {
                return Err(format!(
                    "baseline version {v} is not supported (this groovyfmt reads version {VERSION}); write it again with --write-baseline"
                ));
            }
            None => return Err("not a groovyfmt baseline (no \"version\")".into()),
        }
        let entries = doc.get("entries");
        if !matches!(entries, crate::json::Value::Array(_)) {
            return Err("not a groovyfmt baseline (no \"entries\" list)".into());
        }
        let mut counts = HashMap::new();
        for (i, e) in entries.arr().iter().enumerate() {
            let field = |k: &str| {
                e.get(k)
                    .str()
                    .map(String::from)
                    .ok_or_else(|| format!("entry {i}: no string \"{k}\""))
            };
            let key = (field("path")?, field("rule")?, field("fingerprint")?);
            if e.get("index").u32().is_none() {
                return Err(format!("entry {i}: no \"index\""));
            }
            *counts.entry(key).or_insert(0) += 1;
        }
        Ok(Baseline { counts })
    }

    pub fn load(file: &Path) -> Result<Baseline, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        Baseline::parse(&text).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// Whether the baseline has any violation of `rule` on a line like `line` in `path`
    /// (for `--fix`, which leaves those alone).
    pub fn covers_line(&self, path: &str, rule: &str, line: &str) -> bool {
        self.counts
            .contains_key(&(path.to_string(), rule.to_string(), fingerprint(line)))
    }

    /// Splits `vs`, the violations of `src` at baseline path `path`, into the ones the
    /// baseline doesn't have (returned) and the ones it does (counted), and how many of this
    /// file's baseline entries matched nothing (stale).
    pub fn filter(&self, path: &str, src: &str, vs: Vec<Violation>) -> (Vec<Violation>, usize, usize) {
        let ks = keys(src, &vs);
        let mut matched: HashMap<Key, usize> = HashMap::new();
        let mut kept = Vec::new();
        for (v, (rule, fp, index)) in vs.into_iter().zip(ks) {
            let key = (path.to_string(), rule, fp);
            let have = self.counts.get(&key).copied().unwrap_or(0);
            if index < have {
                *matched.entry(key).or_default() += 1;
            } else {
                kept.push(v);
            }
        }
        let baselined = matched.values().sum();
        let in_file: usize = self
            .counts
            .iter()
            .filter(|((p, _, _), _)| p == path)
            .map(|(_, n)| n)
            .sum();
        (kept, baselined, in_file - baselined)
    }

    /// The paths with entries.
    pub fn paths(&self) -> BTreeSet<&str> {
        self.counts.keys().map(|(p, _, _)| p.as_str()).collect()
    }
}

/// The baseline document: `entries` (path, rule, fingerprint, index) of the files this run
/// linted, whose paths are `linted`, plus `keep`'s entries for every other file, carried over
/// unchanged. Sorted, so the same violations always give the same bytes.
pub fn write(
    mut entries: Vec<(String, String, String, usize)>,
    keep: &Baseline,
    linted: &BTreeSet<String>,
) -> String {
    for ((p, r, f), n) in &keep.counts {
        if !linted.contains(p) {
            entries.extend((0..*n).map(|i| (p.clone(), r.clone(), f.clone(), i)));
        }
    }
    entries.sort();
    let mut s = format!("{{\"version\":{VERSION},\"entries\":[");
    for (i, (p, r, f, n)) in entries.iter().enumerate() {
        s.push_str(if i == 0 { "\n" } else { ",\n" });
        s.push_str(&format!(
            "{{\"path\":{},\"rule\":{},\"fingerprint\":{},\"index\":{n}}}",
            string(p),
            string(r),
            string(f)
        ));
    }
    s.push_str("\n]}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::lint;

    fn kept(b: &Baseline, src: &str) -> (Vec<(&'static str, usize)>, usize, usize) {
        let (vs, held, stale) = b.filter("A.groovy", src, lint(src).unwrap());
        (vs.iter().map(|v| (v.rule, v.line)).collect(), held, stale)
    }

    fn baseline_of(src: &str) -> Baseline {
        let entries = keys(src, &lint(src).unwrap())
            .into_iter()
            .map(|(r, f, i)| ("A.groovy".to_string(), r, f, i))
            .collect();
        Baseline::parse(&write(entries, &Baseline::default(), &BTreeSet::new())).unwrap()
    }

    #[test]
    fn matches_survive_unrelated_edits() {
        let src = "import a.B\nx = 1;\n";
        let b = baseline_of(src);
        assert_eq!(kept(&b, src), (vec![], 2, 0));
        // lines above, re-indented, CR LF: still the same violations
        assert_eq!(kept(&b, "// new\n\nimport a.B\n   x  =  1;\r\n"), (vec![], 2, 0));
        // a new violation is new
        assert_eq!(
            kept(&b, "import a.B\nimport c.D\nx = 1;\n"),
            (vec![("UnusedImport", 2)], 2, 0)
        );
    }

    #[test]
    fn identical_violations_count_by_occurrence() {
        let src = "x = 1;\nx = 1;\nx = 1;\n";
        let b = baseline_of(src);
        // one fixed: the baseline still holds the other two, and one entry is stale
        assert_eq!(kept(&b, "x = 1;\nx = 1\nx = 1;\n"), (vec![], 2, 1));
        // a fourth copy is new, wherever it is
        assert_eq!(
            kept(&b, "x = 1;\nx = 1;\nx = 1;\nx = 1;\n"),
            (vec![("UnnecessarySemicolon", 4)], 3, 0)
        );
    }

    #[test]
    fn renamed_files_appear_new() {
        let src = "import a.B\n";
        let b = baseline_of(src);
        let (vs, held, stale) = b.filter("B.groovy", src, lint(src).unwrap());
        assert_eq!((vs.len(), held, stale), (1, 0, 0));
        assert_eq!(b.filter("A.groovy", "", vec![]).2, 1, "the old path's entry is stale");
    }

    #[test]
    fn the_document_is_sorted_versioned_and_escaped() {
        let entries = vec![
            ("z/\"q\".groovy".to_string(), "R".to_string(), "f".to_string(), 0),
            ("a/../../etc/x".to_string(), "R".to_string(), "f".to_string(), 0),
        ];
        let doc = write(entries, &Baseline::default(), &BTreeSet::new());
        assert_eq!(
            doc,
            "{\"version\":1,\"entries\":[\n{\"path\":\"a/../../etc/x\",\"rule\":\"R\",\"fingerprint\":\"f\",\"index\":0},\n{\"path\":\"z/\\\"q\\\".groovy\",\"rule\":\"R\",\"fingerprint\":\"f\",\"index\":0}\n]}\n"
        );
        // a hostile path reads back as the same string, and is only ever compared
        let b = Baseline::parse(&doc).unwrap();
        assert!(b.paths().contains("a/../../etc/x"));
        assert!(b.paths().contains("z/\"q\".groovy"));
    }

    #[test]
    fn bad_documents_are_errors() {
        for (doc, why) in [
            ("", "parse"),
            ("[]", "version"),
            ("{\"version\":2,\"entries\":[]}", "version 2"),
            ("{\"version\":1}", "entries"),
            ("{\"version\":1,\"entries\":[{\"path\":\"a\"}]}", "rule"),
        ] {
            let e = Baseline::parse(doc).unwrap_err();
            assert!(!e.is_empty(), "{why}");
        }
        assert_eq!(
            Baseline::parse("{\"version\":1,\"entries\":[]}"),
            Ok(Baseline::default())
        );
    }

    #[test]
    fn relative_paths_use_slashes_and_dot_dot() {
        assert_eq!(relative(Path::new("/r/src/A.groovy"), Path::new("/r")), "src/A.groovy");
        assert_eq!(relative(Path::new("/r/A.groovy"), Path::new("/r/sub")), "../A.groovy");
        assert_eq!(relative(Path::new("/r/./x/../A.groovy"), Path::new("/r/")), "A.groovy");
    }
}
