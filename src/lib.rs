pub mod baseline;
pub mod codenarc;
mod codenarc_data;
pub mod config;
pub mod diff;
pub mod fix;
pub mod format;
pub mod json;
pub mod lint;
pub mod lsp;
pub mod report;
mod spacing;
pub mod syntax;
pub mod wrap;

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};
use tree_sitter_language::LanguageFn;

/// Files larger than this are skipped: generated or vendored code, and a guard against
/// a crafted input exhausting memory. `--lsp` leaves documents this large alone too.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

unsafe extern "C" {
    fn tree_sitter_groovy() -> *const ();
}

// SAFETY: tree_sitter_groovy is the entry point of grammar/parser.c, compiled by build.rs. It
// takes no arguments and returns a pointer to the grammar's static TSLanguage, which is what
// from_raw requires. The only unsafe code in groovyfmt outside its tests.
const GROOVY: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_groovy) };

pub fn parse(src: &str) -> Tree {
    // one parser per thread, reused: a file is parsed two to four times
    thread_local! {
        static PARSER: std::cell::RefCell<Parser> = std::cell::RefCell::new({
            let mut p = Parser::new();
            p.set_language(&GROOVY.into())
                .expect("grammar ABI mismatch");
            p
        });
    }
    // The grammar's lexer reads a NUL byte as the end of input, so a string holding one, which
    // Groovy 2.4, 3 and 4 accept, didn't parse (#184). A copy with each NUL as U+0001, also one
    // byte and taken by the same rules, is parsed instead, so every node's range is the same
    // in `src`.
    let src = if src.contains('\0') {
        std::borrow::Cow::Owned(src.replace('\0', "\u{1}"))
    } else {
        std::borrow::Cow::Borrowed(src)
    };
    PARSER.with_borrow_mut(|p| p.parse(&*src, None).expect("parse cancelled"))
}

/// Visit every node in document order. Iterative, so deeply nested input can't overflow the
/// stack; `f` returns false to skip a node's children.
pub fn preorder<'a>(root: Node<'a>, mut f: impl FnMut(Node<'a>) -> bool) {
    let mut c = root.walk();
    loop {
        if f(c.node()) && c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return;
            }
        }
    }
}

/// Source text of a node.
pub fn text<'a>(n: Node, src: &'a str) -> &'a str {
    n.utf8_text(src.as_bytes()).unwrap_or("")
}

/// First ERROR or MISSING node in document order, if any.
pub fn first_error(tree: &Tree) -> Option<Node<'_>> {
    if !tree.root_node().has_error() {
        return None;
    }
    let mut c = tree.walk();
    loop {
        let n = c.node();
        if n.is_error() || n.is_missing() {
            return Some(n);
        }
        if n.has_error() && c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return None;
            }
        }
    }
}

/// A path as groovyfmt prints it (reports, JSON, SARIF, patch headers, errors): on Windows
/// with `/` between components, so output and patches are the same on every platform. A
/// name that isn't valid Unicode shows U+FFFD in place of the bad part.
pub fn display_path(p: &Path) -> String {
    let s = p.display().to_string();
    // `\` can't be part of a file name on Windows, so this changes only separators
    #[cfg(windows)]
    let s = s.replace('\\', "/");
    s
}

/// The characters in `prefix`, line `row` (0-based) up to a byte column: a column as editors
/// count it, so without a UTF-8 BOM at the start of the file, which they don't show.
pub fn chars_before(prefix: &str, row: usize) -> usize {
    let prefix = match row {
        0 => prefix.strip_prefix('\u{feff}').unwrap_or(prefix),
        _ => prefix,
    };
    prefix.chars().count()
}

/// Control characters escaped, so a crafted file name or file content (lint messages quote
/// source text) can't break a report line or inject CI workflow commands such as `::set-env`.
/// The GitHub Actions runner also takes the older `##[command]` form anywhere in a line, so
/// `##[` becomes `##\[`; nothing printed after this (`gh_escape`, `shown`) adds or removes
/// a `\` or `[`, so the sequence can't come back.
pub fn printable(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_debug().to_string()
            } else {
                c.to_string()
            }
        })
        .collect::<String>()
        .replace("##[", "##\\[")
}

/// Groovy sources, Gradle build scripts, and Jenkinsfiles: the names a directory walk takes.
/// The same names the editor plugins treat as Groovy, and an extensionless `Jenkinsfile<Name>`
/// (`JenkinsfileBase`), except a backup (`Jenkinsfile~`) or a
/// `Jenkinsfile.*`, `Jenkinsfile_*` or `Jenkinsfile-*` with a suffix that says it is something
/// else (`Jenkinsfile.md`, `Jenkinsfile_k8s.j2`). A walk also reads the pipeline names other
/// than `Jenkinsfile` and `*.jenkinsfile` first, and takes an extensionless script by its
/// shebang (`is_walked_file`).
pub fn is_groovy_file(path: &Path) -> bool {
    let name = &*path
        .file_name()
        .map_or(std::borrow::Cow::Borrowed(""), |n| n.to_string_lossy());
    [".groovy", ".gradle", ".gvy", ".gy", ".gsh"]
        .iter()
        .any(|ext| name.ends_with(ext))
        || (is_jenkinsfile(name)
            && !name.ends_with('~')
            && !jenkinsfile_suffix(name)
                .and_then(|s| s.rsplit_once('.'))
                .is_some_and(|(_, ext)| NOT_GROOVY.contains(&ext.to_ascii_lowercase().as_str())))
}

/// Whether a directory walk takes the file at `path`: a name `is_groovy_file` takes, and for
/// a `Jenkinsfile.*`, `Jenkinsfile_*`, `Jenkinsfile-*`, `Jenkinsfile<Name>`, `*-Jenkinsfile`,
/// `*_Jenkinsfile` or `*.jenkins` not ending in `.groovy`, contents that are Groovy.
/// Such a name may be a template, a Dockerfile or a note (`Jenkinsfile_template`,
/// `Dockerfile.jenkins`) that would only fail the run, or a pipeline with a syntax error,
/// which the run should report (#222). So one that doesn't parse is still taken unless it
/// reads as something else (`not_groovy`) or is a hidden file (a `.jenkins` marker file);
/// then it is left out without a word, as a `.txt` is.
/// Any other file with no extension (and not ending in `~`) is taken when its first line is a
/// groovy shebang (`is_groovy_shebang`); only its first 256 bytes are read, and files with
/// other extensions are never opened.
/// A `.gsh` is also a GLSL geometry shader: one that starts with a `#version`, `#include`,
/// `#define`, `#ifdef`, `#ifndef` or `#extension` directive, after any blank lines and `//` or
/// `/* */` comments (a license header), is left out (#230, #236, #242).
/// A file named on the command line is not read here, and is taken whatever it holds.
pub fn is_walked_file(path: &Path) -> bool {
    let name = &*path
        .file_name()
        .map_or(std::borrow::Cow::Borrowed(""), |n| n.to_string_lossy());
    if !is_groovy_file(path) {
        return Path::new(name).extension().is_none()
            && !name.ends_with('~')
            && has_groovy_shebang(path);
    }
    if name.ends_with(".gsh") {
        return !glsl_shader(path);
    }
    let parse_gated = (jenkinsfile_suffix(name).is_some()
        || name.ends_with("-Jenkinsfile")
        || name.ends_with("_Jenkinsfile")
        || ends_with_ignore_case(name, ".jenkins"))
        && !name.ends_with(".groovy");
    if !parse_gated {
        return true;
    }
    // too large or not UTF-8 is reported when it is read again, as for any other file
    match std::fs::metadata(path) {
        Ok(m) if m.len() <= MAX_FILE_BYTES => {}
        _ => return true,
    }
    match std::fs::read(path).map(String::from_utf8) {
        Ok(Ok(src)) => {
            let tree = parse(&src);
            !tree.root_node().has_error() || !(name.starts_with('.') || not_groovy(&src, &tree))
        }
        _ => true,
    }
}

/// Whether `src`, which doesn't parse, reads as something other than Groovy: a first line
/// that is a `#!` for another program (a shell script), or a syntax error (of the first
/// `syntax::MAX_REPORTED`) on a line starting with `#` (a Dockerfile, shell or YAML comment)
/// or a template tag (Jinja `{%`, `{#`, ERB/JSP `<%`, Mustache `{{#`, `{{/`). A pipeline
/// with a typo fails on Groovy code, so it is still reported.
fn not_groovy(src: &str, tree: &Tree) -> bool {
    if src.starts_with("#!") && !is_groovy_shebang(src.as_bytes()) {
        return true;
    }
    let (errors, _) = syntax::errors(src, tree);
    errors
        .iter()
        .filter_map(|e| src.lines().nth(e.line - 1))
        .any(|l| {
            let l = l.trim_start();
            ["#", "{%", "{#", "{{#", "{{/", "<%"]
                .iter()
                .any(|m| l.starts_with(m))
        })
}

/// Whether the first line of the file at `path` is a groovy shebang, reading at most 256 bytes.
fn has_groovy_shebang(path: &Path) -> bool {
    head(path, 256).is_some_and(|h| is_groovy_shebang(&h))
}

/// Whether the file at `path` starts with a GLSL preprocessor directive (`#version`,
/// `#include`, `#define`, `#ifdef`, `#ifndef`, `#extension`) after any whitespace and
/// `//` or `/* */` comments, reading at most 64 KiB. Groovy has no `#` there, so such a file
/// is never a Groovy script.
fn glsl_shader(path: &Path) -> bool {
    let Some(h) = head(path, 64 * 1024) else {
        return false;
    };
    let mut rest = &h[..];
    loop {
        rest = rest.trim_ascii_start();
        if let Some(r) = rest.strip_prefix(b"//") {
            rest = r
                .iter()
                .position(|&b| b == b'\n')
                .map_or(&[][..], |i| &r[i + 1..]);
        } else if let Some(r) = rest.strip_prefix(b"/*") {
            let Some(end) = r.windows(2).position(|w| w == b"*/") else {
                return false;
            };
            rest = &r[end + 2..];
        } else {
            // a Veil shader starts with `#include veil:camera` (#242)
            return [
                &b"#version"[..],
                b"#include",
                b"#define",
                b"#ifdef",
                b"#ifndef",
                b"#extension",
            ]
            .iter()
            .any(|d| rest.starts_with(d));
        }
    }
}

/// The first `max` bytes of the file at `path`.
fn head(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(max).read_to_end(&mut head))
        .ok()
        .map(|_| head)
}

/// Whether `src` starts with a shebang line that runs groovy: `#!groovy`, `#!/usr/bin/groovy`,
/// `#!/usr/bin/env groovy`, with or without arguments (`#!/usr/bin/env -S groovy -cp lib`).
/// A first line that isn't UTF-8 (a binary) isn't one.
pub fn is_groovy_shebang(src: &[u8]) -> bool {
    let line = src.split(|&b| b == b'\n').next().unwrap_or_default();
    let Some(Ok(rest)) = line.strip_prefix(b"#!").map(std::str::from_utf8) else {
        return false;
    };
    let program = |w: &str, name: &str| w.rsplit('/').next() == Some(name);
    let mut words = rest.split_whitespace();
    let mut first = words.next();
    if first.is_some_and(|w| program(w, "env")) {
        // `env` options and `NAME=value` assignments come before the program
        first = words.find(|w| !w.starts_with('-') && !w.contains('='));
    }
    first.is_some_and(|w| program(w, "groovy"))
}

/// Suffixes of `Jenkinsfile.*` (and `Jenkinsfile_*.*`, `Jenkinsfile-*.*`) names that a directory walk leaves alone without reading them:
/// docs, data, templates, patches and backup copies, which would not survive being formatted
/// as Groovy.
const NOT_GROOVY: &[&str] = &[
    "md",
    "txt",
    "adoc",
    "rst",
    "html",
    "orig",
    "rej",
    "bak",
    "swp",
    "tmp",
    "yml",
    "yaml",
    "json",
    "xml",
    "properties",
    "sh",
    "bat",
    "ps1",
    "py",
    "j2",
    "tpl",
    "tmpl",
    "template",
    "erb",
    "mustache",
    "hbs",
    "patch",
    "diff",
    "cfg",
    "ini",
    "env",
    "toml",
    "log",
];

/// A pipeline by its name: `Jenkinsfile`, `Jenkinsfile.*`, `Jenkinsfile_*`, `Jenkinsfile-*`
/// (`Jenkinsfile_k8s`), an extensionless `Jenkinsfile<Name>` (`JenkinsfileBase`),
/// `*-Jenkinsfile`, `*_Jenkinsfile` (`rstan-Jenkinsfile`), `*.jenkinsfile` or `*.jenkins` (those two in any case: `ci/release.Jenkinsfile`).
pub fn is_jenkinsfile(name: &str) -> bool {
    name == "Jenkinsfile"
        || jenkinsfile_suffix(name).is_some()
        || name.ends_with("-Jenkinsfile")
        || name.ends_with("_Jenkinsfile")
        || ends_with_ignore_case(name, ".jenkinsfile")
        || ends_with_ignore_case(name, ".jenkins")
}

fn ends_with_ignore_case(name: &str, end: &str) -> bool {
    name.len()
        .checked_sub(end.len())
        .and_then(|i| name.get(i..))
        .is_some_and(|tail| tail.eq_ignore_ascii_case(end))
}

/// What follows `Jenkinsfile` in `Jenkinsfile.*`, `Jenkinsfile_*` or `Jenkinsfile-*`,
/// separator included (`.release`, `_k8s`), or in an extensionless `Jenkinsfile*` going on
/// with a capital (`JenkinsfileSnykScan`, #212; not `Jenkinsfiles`).
fn jenkinsfile_suffix(name: &str) -> Option<&str> {
    name.strip_prefix("Jenkinsfile").filter(|rest| {
        rest.starts_with(['.', '_', '-'])
            || (rest.starts_with(|c: char| c.is_ascii_uppercase()) && !rest.contains('.'))
    })
}

/// Directories skipped while walking: hidden ones (except the CI ones in `CI_DIRS`) and common
/// build/vendor output.
/// A directory named explicitly on the command line is always entered.
///
/// `out` and `target` follow the rule for `build` below, except that they are output in a
/// project with only a `.git` too: under a source root they are packages (#195).
///
/// `build` is Gradle's output directory, but also a fine package name
/// (`src/main/groovy/org/acme/build`) or a plain directory (`jobs/build/x/Jenkinsfile`). Its
/// project is the nearest directory above it holding a Gradle or Maven build file
/// (`BUILD_FILES`) or a `.git`. It is output, and skipped, only when that project has a build
/// file and no source root (`SOURCE_ROOTS`: `src`, a Jenkins library's `vars`, a source set's
/// `groovy`) lies between them. So `sub/build` is skipped even in a subproject with no build
/// file of its own, `lib/src/main/groovy/build`, `test/groovy/org/acme/build` (a source set
/// outside `src`) and `vars/build` are entered, a checkout kept under `~/src` doesn't count,
/// and in a repository with a `.git` but no build file (a Jenkins job repository) `build` is
/// an ordinary directory. Outside any project (no build file or `.git` above it), a source
/// root anywhere in its absolute path makes it a package.
/// The path is absolute so a walk started inside `src` (`cd src/main/groovy; groovyfmt .`)
/// counts.
pub fn is_skipped_dir(path: &Path) -> bool {
    skipped_dir(path).is_some()
}

/// Why `is_skipped_dir` skips `path`, for `-v`.
pub fn skipped_dir(path: &Path) -> Option<&'static str> {
    let name = &*path
        .file_name()
        .map_or(std::borrow::Cow::Borrowed(""), |n| n.to_string_lossy());
    if matches!(name, "build" | "out" | "target") {
        let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut in_src = false;
        let mut output = true;
        for a in full.ancestors().skip(1) {
            let build_file = BUILD_FILES.iter().any(|f| a.join(f).is_file());
            if build_file || a.join(".git").exists() {
                // `out` (IntelliJ) and `target` (Maven) are output in any project
                output = build_file || name != "build";
                break;
            }
            in_src |= a
                .file_name()
                .is_some_and(|n| SOURCE_ROOTS.iter().any(|r| n == *r));
        }
        return (output && !in_src).then_some("build output");
    }
    if name.starts_with('.') && !CI_DIRS.contains(&name) {
        Some("hidden")
    } else if name == "node_modules" {
        Some("dependencies")
    } else {
        None
    }
}

/// Hidden directories a walk enters anyway: where projects keep their CI pipelines and
/// scripts (`.jenkins/ci.jenkins`, `.ci/jenkins/*.groovy`, `.github/scripts/*.groovy`), which
/// are the project's own code (#202). Every other hidden directory is a VCS's, a tool's or a
/// cache (`.git`, `.gradle`, `.idea`, `.m2`), so it stays skipped.
const CI_DIRS: &[&str] = &[
    ".ci",
    ".jenkins",
    ".github",
    ".gitlab",
    ".circleci",
    ".buildkite",
    ".pipelines",
    ".azure-pipelines",
];

/// Directories whose `build`, `out` and `target` packages are source, not output: `src`, a Jenkins shared
/// library's `vars`, and the language directory of a source set (`test/groovy`,
/// `integrationTest/groovy`, `src/main/groovy`), where Gradle never writes its output (#173).
const SOURCE_ROOTS: &[&str] = &["src", "vars", "groovy"];

/// Files that mark a project's root directory, whose `build/` is its output.
const BUILD_FILES: &[&str] = &[
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "pom.xml",
];

/// For the tests that guard against quadratic (or worse) blow-ups: how cost grows with input
/// size, measured so that a loaded machine doesn't fail them. A fixed limit in seconds did:
/// on a shared box at load 15 the linear tests took 3 to 4 times longer, in parallel with the
/// rest of the suite.
#[cfg(test)]
pub(crate) mod scaling {
    use std::time::{Duration, Instant};

    /// The CPU time this thread has used, so other tests and processes don't count:
    /// `clock_gettime` on Linux and macOS, `GetThreadTimes` on Windows (which Wine answers
    /// from the host's per-thread counters). Wall time would count a loaded machine too.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn thread_cpu() -> Option<Duration> {
        use std::ffi::{c_int, c_long};
        #[repr(C)]
        struct Timespec(c_long, c_long);
        unsafe extern "C" {
            fn clock_gettime(clock: c_int, t: *mut Timespec) -> c_int;
        }
        // CLOCK_THREAD_CPUTIME_ID
        const CLOCK: c_int = if cfg!(target_os = "linux") { 3 } else { 16 };
        let mut t = Timespec(0, 0);
        // SAFETY: clock_gettime writes only the timespec it is given.
        (unsafe { clock_gettime(CLOCK, &mut t) } == 0)
            .then(|| Duration::new(t.0 as u64, t.1 as u32))
    }

    #[cfg(windows)]
    fn thread_cpu() -> Option<Duration> {
        unsafe extern "system" {
            fn GetCurrentThread() -> isize;
            fn GetThreadTimes(h: isize, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> i32;
        }
        // creation, exit, kernel and user time: FILETIMEs, two u32 halves read here as one
        // little-endian u64, in 100 ns units
        let mut t = [0u64; 4];
        let p = t.as_mut_ptr();
        // SAFETY: GetThreadTimes writes only the four FILETIMEs it is given, and
        // GetCurrentThread's pseudo-handle needs no closing.
        let ok = unsafe { GetThreadTimes(GetCurrentThread(), p, p.add(1), p.add(2), p.add(3)) };
        (ok != 0).then(|| Duration::from_nanos((t[2] + t[3]) * 100))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    fn thread_cpu() -> Option<Duration> {
        None
    }

    /// How long `reps` calls of `f(n)` take: this thread's CPU time where available, else
    /// wall time, which counts whatever else the machine is doing too, so the fastest of
    /// three tries.
    fn cost(f: &impl Fn(usize), n: usize, reps: usize) -> Duration {
        let run = || (0..reps).for_each(|_| f(n));
        let (cpu, wall) = (thread_cpu(), Instant::now());
        run();
        match (cpu, thread_cpu()) {
            (Some(a), Some(b)) => b.saturating_sub(a),
            _ => (0..2)
                .map(|_| {
                    let t = Instant::now();
                    run();
                    t.elapsed()
                })
                .fold(wall.elapsed(), Duration::min),
        }
    }

    /// The least the smaller measurement may cost: several ticks of the coarsest clock used
    /// (Windows' thread CPU time counts in ticks of up to 15.6 ms), so a tick either way is
    /// noise, not a ratio.
    const MIN_COST: Duration = Duration::from_millis(100);

    /// Assert that `f` on `k` times the input costs at most `bound` times what it costs on
    /// the input: under 8x for 4x the input passes anything linear (or n log n) and fails
    /// anything quadratic (16x). The input starts at `n` and doubles until a run costs
    /// `MIN_COST`, so the comparison is between costs the clock measures well, however fast
    /// the machine, and the growing part of the cost outweighs the fixed one. A failure is
    /// measured again once, so a burst of noise in either run can't fail it.
    pub fn assert_scales(what: &str, f: impl Fn(usize), n: usize, k: usize, bound: u32) {
        assert_scales_up_to(what, f, n, usize::MAX, k, bound);
    }

    /// `assert_scales` for a workload with a hard limit: the input grows only while `k`
    /// times it stays within `max`; past that, each measurement repeats the call until the
    /// smaller one costs `MIN_COST`.
    pub fn assert_scales_up_to(
        what: &str,
        f: impl Fn(usize),
        n: usize,
        max: usize,
        k: usize,
        bound: u32,
    ) {
        scales_up_to(what, &f, n, max, k, bound, cost);
    }

    /// `assert_scales_up_to` with the measurement passed in, so the self-tests below can
    /// count work instead of timing it.
    fn scales_up_to<F: Fn(usize)>(
        what: &str,
        f: &F,
        mut n: usize,
        max: usize,
        k: usize,
        bound: u32,
        cost: impl Fn(&F, usize, usize) -> Duration,
    ) {
        assert!(n * k <= max, "{what}: {k} x {n} is over the limit {max}");
        while n * k * 2 <= max && cost(f, n, 1) < MIN_COST {
            n *= 2;
        }
        let mut reps = 1;
        while cost(f, n, reps) < MIN_COST {
            reps *= 2;
        }
        let mut seen = (Duration::ZERO, Duration::ZERO);
        for _ in 0..2 {
            let small = cost(f, n, reps);
            let big = cost(f, n * k, reps);
            if big <= small * bound {
                return;
            }
            seen = (small, big);
        }
        panic!(
            "{what}: {k}x the input ({n} to {}, {reps} runs each) took {:?} against {:?}, over {bound}x",
            n * k,
            seen.1,
            seen.0
        );
    }

    fn linear(n: usize) {
        std::hint::black_box((0..n as u64 * 1_000).fold(0u64, |a, b| a ^ b));
    }

    fn quadratic(n: usize) {
        let n = n as u64 * 10;
        std::hint::black_box((0..n * n).fold(0u64, |a, b| a ^ b));
    }

    #[test]
    fn linear_work_passes() {
        assert_scales("linear", linear, 1, 4, 8);
        assert_scales_up_to("linear, capped", linear, 1, 8, 4, 8);
    }

    #[test]
    #[should_panic(expected = "over 8x")]
    fn quadratic_work_fails() {
        // from a size far below the clock's resolution, however fast the machine
        assert_scales("quadratic", quadratic, 1, 4, 8);
    }

    #[test]
    #[should_panic(expected = "over 8x")]
    fn quadratic_work_at_a_hard_limit_fails() {
        // can't grow: the repeated calls still show 16x. Counted, not timed: the harness
        // takes the first measurement under the bound, so a single noisy one (seen on macOS
        // CI) let real 16x work pass. One unit per iteration of `quadratic`.
        let work = |_: &fn(usize), n: usize, reps: usize| {
            Duration::from_nanos((n as u64 * 10).pow(2) * reps as u64)
        };
        let f: fn(usize) = quadratic;
        scales_up_to("quadratic, capped", &f, 8, 32, 4, 8, work);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nul_byte_in_a_string_parses() {
        // in every string kind, and in a comment, which already parsed
        let src =
            "def k = 'a' + '\0' + 'b'\ndef j = \"x\0y${k}\"\ndef t = '''\0'''\n// c\0\nprintln k\n";
        let tree = super::parse(src);
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        let mut strings = Vec::new();
        super::preorder(tree.root_node(), |n| {
            if n.kind() == "string_literal" {
                strings.push(super::text(n, src).to_string());
            }
            true
        });
        assert_eq!(strings, ["'a'", "'\0'", "'b'", "\"x\0y${k}\"", "'''\0'''"]);
    }

    /// Every node kind and field name matched as a string in format.rs and lint.rs. A grammar
    /// update that renames one would otherwise silently turn a lint rule into a no-op.
    #[test]
    fn grammar_has_every_kind_and_field_we_match() {
        let lang: tree_sitter::Language = super::GROOVY.into();
        let named = [
            "source_file",
            "identifier",
            "block",
            "class_body",
            "enum_body",
            "closure",
            "method_invocation",
            "command_chain",
            "string_fragment",
            "switch_block",
            "switch_case",
            "labeled_statement",
            "import_declaration",
            "if_statement",
            "while_statement",
            "for_statement",
            "for_in_statement",
            "try_statement",
            "catch_clause",
            "finally_clause",
            "map_literal",
            "map_entry",
            "binary_expression",
            "assignment_expression",
            "parenthesized_expression",
            "string_literal",
            "number_literal",
            "boolean_literal",
            "null_literal",
            "line_comment",
            "block_comment",
            "groovydoc_comment",
            "annotation",
            "gstring_brace_interpolation",
            "gstring_dollar_interpolation",
            // lint/jenkins.rs, lint/gradle.rs, lint/spock.rs
            "pipeline_statement",
            "expression_statement",
            "closure_parameters",
            "named_argument",
            "local_variable_declaration",
            "for_in_statement",
            "do_while_statement",
            "switch_expression",
            "assignment_expression",
            "package_declaration",
            "class_declaration",
            "superclass",
            "method_declaration",
            "quoted_identifier",
            // spacing.rs
            "expression_statement",
            "named_argument",
            "argument_list",
            "formal_parameters",
            "formal_parameter",
            "closure_parameters",
            "type_arguments",
            "type_parameters",
            "array_type",
            "dimension",
            "dimension_expression",
            "method_reference_expression",
            "unary_expression",
            "update_expression",
            "unary_update_expression",
            "spread_arguments",
            "ternary_expression",
            "elvis_expression",
            "cast_expression",
            "parenthesized_type_cast",
            "object_creation_expression",
            "lambda_expression",
            "variable_declarator",
            "local_variable_declaration",
            "field_declaration",
            "multi_assignment_declaration",
            "multi_assignment_expression",
            "switch_default",
            "switch_arrow_case",
            "switch_expression",
            "quoted_identifier",
            "power_expression",
            "membership_expression",
            "instanceof_expression",
            "spaceship_expression",
            "regex_find_expression",
            "regex_match_expression",
            "identity_expression",
            "logical_implication_expression",
            "return_statement",
            "list_literal",
            "do_while_statement",
            "synchronized_statement",
            // lint/basic.rs, lint/imports.rs, lint/unused.rs
            "switch_default",
            "static_initializer",
            "instance_initializer",
            "cast_expression",
            "ternary_expression",
            "elvis_expression",
            "assert_statement",
            "throw_statement",
            "return_statement",
            "break_statement",
            "continue_statement",
            "unary_expression",
            "field_access",
            "subscript_expression",
            "field_declaration",
            "variable_declarator",
            "formal_parameters",
            "formal_parameter",
            "object_creation_expression",
            "list_literal",
            "constructor_declaration",
            "lambda_expression",
            "spread_arguments",
            "spread_map_entry",
            "interface_declaration",
            "enum_declaration",
            "trait_declaration",
            "record_declaration",
            "annotation_type_declaration",
        ];
        for k in named {
            assert_ne!(lang.id_for_node_kind(k, true), 0, "missing node kind {k}");
        }
        for k in [
            ";",
            "=",
            "==",
            "{",
            "}",
            "(",
            ")",
            "[",
            "]",
            ",",
            ":",
            "?",
            "->",
            "::",
            "<",
            ">",
            "|",
            "||",
            "else",
            "catch",
            "finally",
            "while",
            "if",
            "for",
            "switch",
            "synchronized",
            "<<",
            "&&",
            "!=",
            "%",
            "private",
            "static",
        ] {
            assert_ne!(lang.id_for_node_kind(k, false), 0, "missing token {k}");
        }
        let fields = [
            "name",
            "alias",
            "body",
            "parameter",
            "condition",
            "consequence",
            "alternative",
            "key",
            "value",
            "operator",
            "left",
            "right",
            "label",
            "receiver",
            "argument",
            "function",
            "arguments",
            "closure",
            "statement",
            "base",
            "type",
            "return_type",
            "parameters",
            "object",
            "field",
            "operand",
            "element",
        ];
        for f in fields {
            assert!(lang.field_id_for_name(f).is_some(), "missing field {f}");
        }
    }
}
