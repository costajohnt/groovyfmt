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
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Bumped on any change to the file format; another version is an error.
pub const VERSION: u32 = 1;

/// Where `--write-baseline` writes when nothing else names a file.
pub const DEFAULT_FILE: &str = ".groovyfmt-baseline.json";

/// (rule, fingerprint).
type Key = (String, String);

/// More entries than a 10 MB file can hold (each takes over 40 bytes); a bound on what one
/// can make groovyfmt allocate.
pub const MAX_ENTRIES: usize = 250_000;

#[derive(Debug, Default, PartialEq)]
pub struct Baseline {
    /// By path, how many violations each (rule, fingerprint) has, and the file's total: each
    /// lookup is a hash lookup, however many entries the baseline has.
    files: HashMap<String, (HashMap<Key, usize>, usize)>,
}

/// FNV-1a, 64 bits: stable across runs, platforms and Rust versions, unlike std's hasher.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The fingerprint of a line: its text with every whitespace run made one space, the ends
/// trimmed and a final `;` dropped (so re-indenting, CR LF, trailing spaces and `--fix`
/// removing the `;` don't change it), hashed.
pub fn fingerprint(line: &str) -> String {
    let collapsed: Vec<&str> = line.split_whitespace().collect();
    let text = collapsed.join(" ");
    format!("{:016x}", fnv1a(text.trim_end_matches(';').trim_end()))
}

/// A file's violations as (rule, fingerprint, index).
pub type Keys = Vec<(String, String, usize)>;

/// Each violation's (rule, fingerprint, index), in the order given.
pub fn keys(src: &str, vs: &[Violation]) -> Keys {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut order: Vec<usize> = (0..vs.len()).collect();
    order.sort_by_key(|&i| (vs[i].line, vs[i].col));
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut out = vec![(String::new(), String::new(), 0); vs.len()];
    for i in order {
        let fp = fingerprint(lines.get(vs[i].line - 1).copied().unwrap_or(""));
        let n = seen
            .entry((vs[i].rule.to_string(), fp.clone()))
            .or_default();
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
        if !matches!(entries, crate::json::Value::Arr(_)) {
            return Err("not a groovyfmt baseline (no \"entries\" list)".into());
        }
        if entries.arr().len() > MAX_ENTRIES {
            return Err(format!("more than {MAX_ENTRIES} entries"));
        }
        let mut files: HashMap<String, (HashMap<Key, usize>, usize)> = HashMap::new();
        for (i, e) in entries.arr().iter().enumerate() {
            let field = |k: &str| {
                e.get(k)
                    .str()
                    .map(String::from)
                    .ok_or_else(|| format!("entry {i}: no string \"{k}\""))
            };
            let (path, key) = (field("path")?, (field("rule")?, field("fingerprint")?));
            if e.get("index").u32().is_none() {
                return Err(format!("entry {i}: no \"index\""));
            }
            let (counts, total) = files.entry(path).or_default();
            *counts.entry(key).or_insert(0) += 1;
            *total += 1;
        }
        Ok(Baseline { files })
    }

    /// Read `file` as source files are read: a regular file (checked on the opened handle,
    /// so not a directory, FIFO or device), at most `MAX_FILE_BYTES`, UTF-8. Errors name the
    /// file and the problem, never its content.
    pub fn load(file: &Path) -> Result<Baseline, String> {
        let err = |e: &dyn std::fmt::Display| format!("{}: {e}", crate::display_path(file));
        let not_file = || err(&"the baseline is not a regular file");
        // Windows refuses to open a directory at all, with "Access is denied"
        let f = std::fs::File::open(file)
            .map_err(|e| if file.is_dir() { not_file() } else { err(&e) })?;
        let meta = f.metadata().map_err(|e| err(&e))?;
        if !meta.is_file() {
            return Err(not_file());
        }
        let mut bytes = Vec::new();
        f.take(crate::MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| err(&e))?;
        if bytes.len() as u64 > crate::MAX_FILE_BYTES {
            return Err(err(&"baseline file is too large (over 10 MB)"));
        }
        let text = String::from_utf8(bytes).map_err(|_| err(&"the baseline is not UTF-8"))?;
        Baseline::parse(&text).map_err(|e| err(&e))
    }

    /// Whether the baseline has any violation of `rule` on a line like `line` in `path`
    /// (for `--fix`, which leaves those alone).
    pub fn covers_line(&self, path: &str, rule: &str, line: &str) -> bool {
        self.files
            .get(path)
            .is_some_and(|(counts, _)| counts.contains_key(&(rule.to_string(), fingerprint(line))))
    }

    /// Splits `vs`, the violations of `src` at baseline path `path`, into the ones the
    /// baseline doesn't have (returned) and the ones it does (counted), and how many of this
    /// file's baseline entries matched nothing (stale).
    pub fn filter(
        &self,
        path: &str,
        src: &str,
        vs: Vec<Violation>,
    ) -> (Vec<Violation>, usize, usize) {
        let Some((counts, in_file)) = self.files.get(path) else {
            return (vs, 0, 0);
        };
        let ks = keys(src, &vs);
        let (mut kept, mut baselined) = (Vec::new(), 0);
        for (v, (rule, fp, index)) in vs.into_iter().zip(ks) {
            if index < counts.get(&(rule, fp)).copied().unwrap_or(0) {
                baselined += 1;
            } else {
                kept.push(v);
            }
        }
        (kept, baselined, in_file - baselined)
    }

    /// The paths with entries.
    pub fn paths(&self) -> BTreeSet<&str> {
        self.files.keys().map(String::as_str).collect()
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
    for (p, (counts, _)) in &keep.files {
        if !linted.contains(p) {
            for ((r, f), n) in counts {
                entries.extend((0..*n).map(|i| (p.clone(), r.clone(), f.clone(), i)));
            }
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

/// Check that `path`, a baseline named by the config whose directory is `root`, stays in
/// `root`: under it once normalized, and not through a symbolic link below it that leads out
/// of it. A pull request controls the config, so it must not make groovyfmt read or write
/// anywhere else; a path given on the command line is the user's choice and isn't checked.
/// Only existing components can be checked; one created between this check and the write
/// needs a concurrent local attacker, as for formatting.
pub fn confined(root: &Path, path: &Path) -> Result<(), String> {
    let outside = || {
        format!(
            "{}: the config's baseline must stay inside the config's directory",
            crate::display_path(path)
        )
    };
    let rel = path.strip_prefix(root).map_err(|_| outside())?;
    let real_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut p = PathBuf::from(root);
    for c in rel.components() {
        if !matches!(c, Component::Normal(_)) {
            return Err(outside());
        }
        p.push(c);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => {
                let real = std::fs::canonicalize(&p).map_err(|_| outside())?;
                if !real.starts_with(&real_root) {
                    return Err(outside());
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
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
        assert_eq!(
            kept(&b, "// new\n\nimport a.B\n   x  =  1;\r\n"),
            (vec![], 2, 0)
        );
        // a new violation is new
        assert_eq!(
            kept(&b, "import a.B\nimport c.D\nx = 1;\n"),
            (vec![("UnusedImport", 2)], 2, 0)
        );
    }

    #[test]
    fn a_fixed_semicolon_leaves_the_line_matched() {
        // `--fix` of the `;` changes the line other reports on it are fingerprinted by
        assert_eq!(fingerprint("def x = 1;"), fingerprint("  def x = 1 "));
        assert_ne!(fingerprint("def x = 1"), fingerprint("def x = 2"));
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
        assert_eq!(
            b.filter("A.groovy", "", vec![]).2,
            1,
            "the old path's entry is stale"
        );
    }

    #[test]
    fn the_document_is_sorted_versioned_and_escaped() {
        let entries = vec![
            (
                "z/\"q\".groovy".to_string(),
                "R".to_string(),
                "f".to_string(),
                0,
            ),
            (
                "a/../../etc/x".to_string(),
                "R".to_string(),
                "f".to_string(),
                0,
            ),
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

    #[cfg(unix)]
    #[test]
    fn a_config_baseline_stays_in_the_config_directory() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("groovyfmt-confined-{}", std::process::id()));
        let out = root.with_extension("outside");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(root.join("ci")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        // the normal case, existing or not
        assert_eq!(confined(&root, &root.join("b.json")), Ok(()));
        assert_eq!(confined(&root, &root.join("ci/new/b.json")), Ok(()));
        // a symlinked parent or target that leads out of the tree
        symlink(&out, root.join("escape")).unwrap();
        assert!(confined(&root, &root.join("escape/b.json")).is_err());
        symlink(out.join("x.yml"), root.join("link.json")).unwrap();
        assert!(confined(&root, &root.join("link.json")).is_err());
        // a link that stays inside is fine
        symlink(root.join("ci"), root.join("ci2")).unwrap();
        assert_eq!(confined(&root, &root.join("ci2/b.json")), Ok(()));
        // a path outside the root, or one that climbs out of it
        assert!(confined(&root, &out.join("b.json")).is_err());
        assert!(confined(&root, &root.join("../b.json")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn loading_is_bounded_and_never_echoes_the_file() {
        let dir = std::env::temp_dir().join(format!("groovyfmt-bload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // a directory is not a baseline
        assert!(
            Baseline::load(&dir)
                .unwrap_err()
                .contains("not a regular file")
        );
        // nor is a file over the size cap
        let big = dir.join("big.json");
        std::fs::write(&big, vec![b' '; (crate::MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert!(Baseline::load(&big).unwrap_err().contains("too large"));
        // a file that isn't a baseline: the error names the file and the problem only
        let secret = dir.join("passwd");
        std::fs::write(&secret, "root:x:0:0:SECRET:/root:/bin/sh\n").unwrap();
        let e = Baseline::load(&secret).unwrap_err();
        assert!(!e.contains("SECRET") && !e.contains("root:x"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_huge_baseline_matches_in_linear_time() {
        // n entries over n / 50 files, each file then matched: a scan of every entry per
        // file (the first version) grows with the square of n
        let run = |n: usize| {
            let files = n / 50;
            let entries: Vec<_> = (0..n)
                .map(|i| {
                    (
                        format!("f{}.groovy", i % files),
                        "R".to_string(),
                        format!("{i:016x}"),
                        0,
                    )
                })
                .collect();
            let doc = write(entries, &Baseline::default(), &BTreeSet::new());
            let b = Baseline::parse(&doc).unwrap();
            let mut stale = 0;
            for i in 0..files {
                stale += b.filter(&format!("f{i}.groovy"), "", Vec::new()).2;
            }
            assert_eq!(stale, n);
        };
        crate::scaling::assert_scales_up_to("baseline matching", run, 25_000, MAX_ENTRIES, 4, 8);
        // and the entry count is bounded
        let too_many = format!(
            "{{\"version\":1,\"entries\":[{}]}}",
            vec!["{}"; MAX_ENTRIES + 1].join(",")
        );
        assert!(
            Baseline::parse(&too_many)
                .unwrap_err()
                .contains("more than")
        );
    }

    #[test]
    fn relative_paths_use_slashes_and_dot_dot() {
        assert_eq!(
            relative(Path::new("/r/src/A.groovy"), Path::new("/r")),
            "src/A.groovy"
        );
        assert_eq!(
            relative(Path::new("/r/A.groovy"), Path::new("/r/sub")),
            "../A.groovy"
        );
        assert_eq!(
            relative(Path::new("/r/./x/../A.groovy"), Path::new("/r/")),
            "A.groovy"
        );
    }
}
