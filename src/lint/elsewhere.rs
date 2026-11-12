//! Uses of a private member from another file. Groovy doesn't enforce `private`, so a
//! private method or field can be called or read from anywhere: `it.generateMethods()`,
//! `Constants.DEFAULT_TIMEOUT`, a method of a `@Mixin` class called by the class mixing it
//! in. UnusedPrivateMethod and UnusedPrivateField look at one file, so before one of their
//! reports is kept, the rest of the project is searched for the name.
//!
//! The project is the nearest directory at or above the file holding `.git` or a config
//! file, so it doesn't depend on which files were named on the command line: the same
//! file gets the same answer from `--lint .`, `--lint one/File.groovy`, `--changed-since`
//! and the LSP. Its Groovy files are found as a directory walk finds them (no symlinks,
//! skipped directories and `exclude` patterns honored) and scanned once per run, only when
//! some file has a report of these rules. Matching is by name only (see `scan`). When the
//! walk hits a cap the rules stay quiet for the whole project rather than report what the
//! unscanned files might use.

use super::Violation;
use super::unused::{lcfirst, property};
use crate::config::{self, Config, Resolver};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The most files and bytes scanned per project; past either (or a file over
/// `MAX_FILE_BYTES`), its reports are dropped.
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

const RULES: [&str; 2] = ["UnusedPrivateMethod", "UnusedPrivateField"];

/// How a word appears: after `.`, `?.`, `*.`, `.&`, `.@` or `::` (`MEMBER`); as a whole
/// string (`'name'`, as `getProperty('name')` or `containsKey('name')` use it);
/// called without a receiver (`name(` or `name {`, as for a `@Mixin`, trait or `@Delegate`
/// method); as the name a type declaration declares; or otherwise.
const MEMBER: u8 = 1;
const STRING: u8 = 2;
const CALL: u8 = 4;
const BARE: u8 = 8;
const DECL: u8 = 16;

/// The projects' indexes, each built the first time a report needs it.
pub struct Projects {
    /// `--config`: its `exclude` decides for every file, as in a directory walk
    fixed: Option<Config>,
    cache: Mutex<HashMap<PathBuf, Arc<OnceLock<Index>>>>,
}

#[derive(Default)]
struct Index {
    files: Vec<PathBuf>,
    /// For each word, the files it appears in (indexes into `files`, ascending) and the ways
    /// it appears there (`MEMBER`, `STRING`, ...).
    words: HashMap<Box<str>, Vec<(u32, u8)>>,
    /// A cap was hit, so a name may be used in a file that wasn't scanned.
    capped: bool,
}

impl Projects {
    pub fn new(fixed: Option<Config>) -> Projects {
        Projects {
            fixed,
            cache: Mutex::default(),
        }
    }

    /// Forget every index, for when files may have changed (the LSP, on save).
    pub fn clear(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Drop the UnusedPrivateMethod and UnusedPrivateField reports in `vs` (of `src`, the
    /// text of the file at `path`) whose name another file of the project uses.
    pub fn filter(&self, path: &Path, src: &str, vs: &mut Vec<Violation>) {
        if !vs.iter().any(|v| RULES.contains(&v.rule)) {
            return;
        }
        let file = config::normalize(&std::env::current_dir().unwrap_or_default().join(path));
        let Some(root) = root_of(&file) else {
            return;
        };
        let cell = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(root.clone())
            .or_default()
            .clone();
        let idx = cell.get_or_init(|| Index::build(&root, self.fixed.clone()));
        let me = idx.position(&root, &file);
        vs.retain(|v| {
            let Some(name) = RULES
                .contains(&v.rule)
                .then(|| src.get(v.span.clone()))
                .flatten()
            else {
                return true;
            };
            let mut names = vec![name.to_string()];
            // `getFoo()` is also called as the property `foo`
            if v.rule == "UnusedPrivateMethod"
                && let Some(p) = property(name)
            {
                names.extend([lcfirst(p), p.to_string()]);
            }
            // a script's methods belong to the class named after the file
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let owner = v.owner.as_deref().unwrap_or(stem);
            // a field is read with a receiver or by name; a method also called bare
            let kinds = if v.rule == "UnusedPrivateField" {
                MEMBER | STRING
            } else {
                MEMBER | STRING | CALL
            };
            !idx.capped
                && !names
                    .iter()
                    .any(|n| idx.used_elsewhere(n, kinds, owner, me))
        });
    }
}

/// The nearest directory above `file` holding `.git` or a config file.
fn root_of(file: &Path) -> Option<PathBuf> {
    file.ancestors().skip(1).find_map(|d| {
        (d.join(".git").exists() || config::FILE_NAMES.iter().any(|n| d.join(n).is_file()))
            .then(|| d.to_path_buf())
    })
}

impl Index {
    fn build(root: &Path, fixed: Option<Config>) -> Index {
        let mut res = Resolver::new(fixed, None);
        let mut files = Vec::new();
        let mut bytes = 0;
        let mut capped = false;
        walk(root, &mut res, &mut files, &mut bytes, &mut capped);
        let mut idx = Index {
            capped,
            ..Index::default()
        };
        if capped {
            return idx;
        }
        for (i, f) in files.iter().enumerate() {
            // a file that went away or can't be read uses nothing
            let Ok(src) = std::fs::read(f) else { continue };
            let i = u32::try_from(i).unwrap_or(u32::MAX - 1);
            scan(&src, |w, kind| {
                let Ok(w) = std::str::from_utf8(w) else {
                    return;
                };
                let e = match idx.words.get_mut(w) {
                    Some(e) => e,
                    None => idx.words.entry(w.into()).or_default(),
                };
                match e.last_mut() {
                    Some((f, k)) if *f == i => *k |= kind,
                    _ => e.push((i, kind)),
                }
            });
        }
        idx.files = files;
        idx
    }

    /// `file`'s index in `files`, also when it was named through a symlink.
    fn position(&self, root: &Path, file: &Path) -> Option<u32> {
        let at = |f: &Path| self.files.iter().position(|p| p == f);
        let found = at(file).or_else(|| {
            let real = std::fs::canonicalize(file).ok()?;
            at(&root.join(real.strip_prefix(std::fs::canonicalize(root).ok()?).ok()?))
        });
        found.and_then(|i| u32::try_from(i).ok())
    }

    /// Whether `name` appears in one of the ways in `kinds` in a file other than `me` that
    /// names `owner`, the type declaring it (or its bean name, `fooService` for
    /// `FooService`). Not in a file that declares a type of that name itself: a copy of the
    /// class, or one of the same name in another package.
    fn used_elsewhere(&self, name: &str, kinds: u8, owner: &str, me: Option<u32>) -> bool {
        let none = Vec::new();
        let named = |w: &str| self.words.get(w).unwrap_or(&none);
        let (a, b) = (named(owner), named(&lcfirst(owner)));
        let names_owner = |f: u32| {
            [a, b].iter().any(|ps| {
                ps.binary_search_by_key(&f, |x| x.0)
                    .is_ok_and(|i| ps[i].1 & DECL == 0)
            })
        };
        named(name)
            .iter()
            .any(|&(f, k)| k & kinds != 0 && Some(f) != me && names_owner(f))
    }
}

/// The Groovy files under `dir` as a directory walk takes them: never through a symlink,
/// not in skipped or excluded directories, and only regular files.
fn walk(
    dir: &Path,
    res: &mut Resolver,
    out: &mut Vec<PathBuf>,
    bytes: &mut u64,
    capped: &mut bool,
) {
    let Ok(cfg) = res.inside(dir) else {
        // a config that can't be read: nothing below it is linted either
        return;
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if *capped {
            return;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            if !crate::is_skipped_dir(&p) && !cfg.excluded(&p) {
                walk(&p, res, out, bytes, capped);
            }
        } else if meta.is_file() && crate::is_groovy_file(&p) && !cfg.excluded(&p) {
            *bytes += meta.len();
            out.push(p);
            *capped =
                out.len() > MAX_FILES || *bytes > MAX_BYTES || meta.len() > crate::MAX_FILE_BYTES;
        }
    }
}

/// Calls `f` with each word of `src` outside comments and how it appears. In a string, an
/// interpolated name (`"$name"`, `"${a.name}"`) counts as code and a string that is one
/// word as `STRING`; other words in strings are prose and skipped. Slashy strings read as
/// code.
fn scan(src: &[u8], mut f: impl FnMut(&[u8], u8)) {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let (n, mut i) = (src.len(), 0);
    // the closing quote of the string we are in, empty in code
    let mut close: &[u8] = b"";
    // the word before, when it was a bare one in code
    let mut prev: &[u8] = b"";
    while i < n {
        let c = src[i];
        let rest = &src[i..];
        if close.is_empty() {
            if rest.starts_with(b"//") {
                i += rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
                continue;
            }
            if rest.starts_with(b"/*") {
                i += rest[2..]
                    .windows(2)
                    .position(|w| w == b"*/")
                    .map_or(rest.len(), |k| k + 4);
                continue;
            }
            if c == b'\'' || c == b'"' {
                close = [&b"'''"[..], b"\"\"\"", b"'", b"\""]
                    .into_iter()
                    .find(|q| rest.starts_with(q))
                    .unwrap_or(b"'");
                i += close.len();
                continue;
            }
        } else if c == b'\\' {
            i += 2;
            continue;
        } else if rest.starts_with(close) {
            i += close.len();
            close = b"";
            continue;
        } else if c == b'\n' && close.len() == 1 {
            // a one-line string can't span lines; whatever this was, it ended
            close = b"";
        }
        if !word(c) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && word(src[i]) {
            i += 1;
        }
        if c.is_ascii_digit() {
            continue;
        }
        let before = src[..start].trim_ascii_end();
        let before = before
            .strip_suffix(b"&")
            .or_else(|| before.strip_suffix(b"@"))
            .unwrap_or(before);
        let after = src[i..].iter().find(|&&b| b != b' ' && b != b'\t');
        // `foo(` and `foo {`, but not `void foo(`, which declares it
        let called = matches!(after, Some(b'(' | b'{'))
            && (!before.last().is_some_and(|&b| word(b))
                || matches!(
                    prev,
                    b"return" | b"assert" | b"throw" | b"else" | b"in" | b"case"
                ));
        // `"$name"` and `"${name}"` are code, a variable more often than a member
        let interpolated = before.ends_with(b"$") || before.ends_with(b"${");
        let quote = |b: Option<&u8>| matches!(b, Some(b'\'' | b'"'));
        let kind = if before.ends_with(b".") || before.ends_with(b"::") {
            MEMBER
        } else if !close.is_empty() && !interpolated {
            // a name used by name is the whole string (`'name'`); other words are prose
            if !(quote(src.get(start.wrapping_sub(1))) && quote(src.get(i))) {
                continue;
            }
            STRING
        } else if matches!(
            prev,
            b"class" | b"interface" | b"trait" | b"enum" | b"record"
        ) {
            DECL
        } else if called && close.is_empty() {
            CALL
        } else {
            BARE
        };
        prev = if kind == BARE { &src[start..i] } else { b"" };
        f(&src[start..i], kind);
    }
}
