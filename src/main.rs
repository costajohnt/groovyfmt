use groovyfmt::MAX_FILE_BYTES;
use groovyfmt::baseline::{self, Baseline};
use groovyfmt::config::{self, Config, Resolver};
use groovyfmt::diff;
use groovyfmt::fix::{self, Fix};
use groovyfmt::format::{Outcome, Style, format};
use groovyfmt::lint::{self, RULES};
use groovyfmt::report::{self, Diagnostic, FileReport};
use groovyfmt::syntax::{self, SyntaxError};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

const HELP: &str = "\
groovyfmt: formatter and linter for Groovy, Gradle build scripts, and Jenkinsfiles

Usage: groovyfmt [OPTIONS] [PATH]...

PATHs are files or directories (default: the current directory). Directories are
searched for *.groovy, *.gradle, *.gvy, *.gy, *.gsh and Jenkinsfile, skipping
hidden directories, build, out, target and node_modules, and anything matching
the config file's `exclude` patterns (which also apply to files named here).
Each file uses the nearest .groovyfmt.toml / groovyfmt.toml in its directory or
above it; a nested config replaces its parent's, nothing is merged.

Modes:
      (default)            Format files in place
      --check              List files that would be reformatted; exit 1 if any
      --lint               Report lint violations; exit 1 if any
      --syntax             Only parse: report each place groovyfmt could not parse;
                           exit 1 if any file has one
      --stdin              Format stdin to stdout (for editors)
      --stdin-filepath <PATH>
                           With --stdin: find the config as if formatting PATH (which
                           need not exist), name it in messages, and if PATH is
                           excluded, echo stdin unchanged
      --lsp                Serve the Language Server Protocol on stdin/stdout (for
                           editors and coding agents): formatting, diagnostics and
                           fixes, each file with its own config
      --suggest-style      Try a few indent styles without writing anything and print
                           the .groovyfmt.toml lines for the one that changes the
                           fewest files (a table of all of them goes to stderr)

Options:
      --config <FILE>      Use FILE for every file instead of the nearest config
      --indent <N>         Spaces per indent level, or the tab width with use_tabs
                           (overrides the config file)
      --output-format <F>  text (default), github (GitHub Actions annotations), json
                           (--check, --lint, --syntax, in place: one JSON document on
                           stdout, schema in the README) or sarif (--lint, --syntax)
      --rules              List lint rules and whether each is on, then exit
      --explain <RULE>     Explain a lint rule: why it matters, an example, whether
                           --fix fixes it and how to turn it off
      --strict             List skipped files (unparseable, not UTF-8, too large) and
                           exit 1 if there are any, so CI can't pass on unchecked files
      --diff               With --check, or --lint --fix: print a unified diff of the
                           changes (for git apply / patch -p1) and write nothing
      --lines <A-B>        Change only lines A to B (repeatable; one file or --stdin)
      --changed-since <REV>
                           Only the files that differ from git revision REV, or are
                           untracked (in the working directory, whole files)
      --baseline <FILE>    With --lint: report only violations not recorded in FILE
                           (overrides the config's `baseline`; '' for none)
      --write-baseline[=FILE]
                           With --lint: record the current violations in FILE (default:
                           --baseline, the config's `baseline`, .groovyfmt-baseline.json)
      --fix                With --lint: remove unused, duplicate and same-package
                           imports and unnecessary semicolons in place (violations marked
                           \"(fixable)\"), then report what remains
  -v, --verbose            Also report skipped files, where each one failed to parse,
                           and timing
  -h, --help               Print help
  -V, --version            Print version
      --                   Treat every later argument as a PATH

Formatting changes whitespace only and never touches a file it can't parse, or
one whose tokens would change. Exit status: 0 clean, 1 files to format or lint
violations found (or, with --strict, files skipped), 2 error.
";

#[derive(Default)]
struct Args {
    mode: Mode,
    paths: Vec<PathBuf>,
    config: Option<PathBuf>,
    stdin_filepath: Option<PathBuf>,
    indent: Option<usize>,
    github: bool,
    /// `--output-format json` or `sarif`: one document on stdout instead of report lines
    machine: Option<Machine>,
    /// `--lint --fix`
    fix: bool,
    /// `--diff`: print patches instead of names (with `--check`) or writing (`--lint --fix`)
    diff: bool,
    /// `--lines`: only these lines (1-based, inclusive) change
    lines: Vec<(usize, usize)>,
    /// `--changed-since`: only files that differ from this git revision, or are untracked
    changed_since: Option<String>,
    verbose: bool,
    strict: bool,
    /// `--baseline`: the baseline for every file instead of the config's; empty for none
    baseline: Option<PathBuf>,
    /// `--write-baseline`, with the file it names if any
    write_baseline: Option<Option<PathBuf>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Machine {
    Json,
    Sarif,
}

#[derive(Default, PartialEq)]
enum Mode {
    #[default]
    Write,
    Check,
    Lint,
    Syntax,
    Stdin,
    Rules,
    Explain(String),
    Lsp,
    SuggestStyle,
    // development diagnostics, not in --help
    Stats,
    LintStats,
    Errors,
    Tree,
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("groovyfmt: {}", printable(&msg.to_string()));
    std::process::exit(2);
}

/// Set when a directory couldn't be read during the walk, or an argument was refused; the
/// run then exits 2.
static WALK_FAILED: AtomicBool = AtomicBool::new(false);

/// One line to stdout. Write errors are ignored, so `groovyfmt --check | head -1` stops
/// printing quietly instead of panicking; the exit status still reflects the results.
fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

fn parse_args() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    let set_mode = |a: &mut Args, m: Mode| {
        // --lint --stats is the one allowed combination
        a.mode = match (std::mem::take(&mut a.mode), m) {
            (Mode::Write, m) => m,
            (Mode::Lint, Mode::Stats) | (Mode::Stats, Mode::Lint) => Mode::LintStats,
            _ => die(
                "--check, --lint, --syntax, --stdin, --rules and --suggest-style can't be combined",
            ),
        };
    };
    let mut only_paths = false;
    while let Some(arg) = it.next() {
        // `--` ends the options: everything after it is a path, even `-x` or `--check`
        if only_paths {
            a.paths.push(PathBuf::from(arg));
            continue;
        }
        if arg == "--" {
            only_paths = true;
            continue;
        }
        let (flag, mut inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| {
            inline
                .take()
                .or_else(|| it.next())
                .unwrap_or_else(|| die(format!("{name} needs a value")))
        };
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("groovyfmt {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--check" => set_mode(&mut a, Mode::Check),
            "--lint" => set_mode(&mut a, Mode::Lint),
            "--syntax" => set_mode(&mut a, Mode::Syntax),
            "--stdin" => set_mode(&mut a, Mode::Stdin),
            "--rules" => set_mode(&mut a, Mode::Rules),
            "--explain" => {
                let rule = value("--explain");
                set_mode(&mut a, Mode::Explain(rule));
            }
            "--suggest-style" => set_mode(&mut a, Mode::SuggestStyle),
            "--lsp" => set_mode(&mut a, Mode::Lsp),
            "--stats" => set_mode(&mut a, Mode::Stats),
            "--errors" => set_mode(&mut a, Mode::Errors),
            "--tree" => set_mode(&mut a, Mode::Tree),
            "-v" | "--verbose" => a.verbose = true,
            "--strict" => a.strict = true,
            "--fix" => a.fix = true,
            "--diff" => a.diff = true,
            "--lines" => {
                let v = value("--lines");
                let range = v
                    .split_once('-')
                    .and_then(|(s, e)| Some((s.parse().ok()?, e.parse().ok()?)))
                    .filter(|&(s, e): &(usize, usize)| 1 <= s && s <= e);
                a.lines.push(range.unwrap_or_else(|| {
                    die(format!(
                        "--lines takes START-END, 1 <= START <= END, not `{v}`"
                    ))
                }));
            }
            "--changed-since" => a.changed_since = Some(value("--changed-since")),
            "--baseline" => a.baseline = Some(PathBuf::from(value("--baseline"))),
            // only `--write-baseline=FILE`: a separate word would read as a path to lint
            "--write-baseline" => a.write_baseline = Some(inline.take().map(PathBuf::from)),
            "--config" => a.config = Some(PathBuf::from(value("--config"))),
            "--stdin-filepath" => a.stdin_filepath = Some(PathBuf::from(value("--stdin-filepath"))),
            "--indent" => {
                let v = value("--indent");
                a.indent = Some(
                    v.parse()
                        .ok()
                        .filter(|n| (1..=16).contains(n))
                        .unwrap_or_else(|| die("--indent must be 1 to 16")),
                );
            }
            "--output-format" => match value("--output-format").as_str() {
                "text" => (a.github, a.machine) = (false, None),
                "github" => (a.github, a.machine) = (true, None),
                "json" => (a.github, a.machine) = (false, Some(Machine::Json)),
                "sarif" => (a.github, a.machine) = (false, Some(Machine::Sarif)),
                other => die(format!(
                    "unknown --output-format `{other}` (text, github, json or sarif)"
                )),
            },
            f if f.starts_with('-') && f != "-" => {
                die(format!("unknown option `{f}` (see --help)"))
            }
            _ => a.paths.push(PathBuf::from(arg)),
        }
        // `--check=false` would otherwise turn check on
        if inline.is_some() {
            die(format!("{flag} doesn't take a value"));
        }
    }
    if a.stdin_filepath.is_some() && a.mode != Mode::Stdin {
        die("--stdin-filepath needs --stdin");
    }
    if a.mode == Mode::Stdin && !a.paths.is_empty() {
        die("--stdin reads stdin and takes no paths (name the file with --stdin-filepath)");
    }
    match (a.machine, &a.mode) {
        (Some(Machine::Json), Mode::Write | Mode::Check | Mode::Lint | Mode::Syntax)
        | (Some(Machine::Sarif), Mode::Lint | Mode::Syntax)
        | (None, _) => {}
        (Some(Machine::Json), _) => {
            die("--output-format json works with --check, --lint, --syntax and formatting in place")
        }
        (Some(Machine::Sarif), _) => die("--output-format sarif works with --lint and --syntax"),
    }
    if a.fix && a.mode != Mode::Lint {
        die("--fix needs --lint");
    }
    if (a.baseline.is_some() || a.write_baseline.is_some()) && a.mode != Mode::Lint {
        die("--baseline and --write-baseline need --lint");
    }
    if a.write_baseline.is_some() && (a.fix || a.machine.is_some() || a.github) {
        die("--write-baseline can't be combined with --fix or --output-format");
    }
    if a.diff && !(a.mode == Mode::Check || a.fix) {
        die("--diff works with --check and --lint --fix");
    }
    if a.diff && (a.github || a.machine.is_some()) {
        die("--diff prints a patch and can't be combined with --output-format");
    }
    if !a.lines.is_empty()
        && !(a.mode == Mode::Stdin
            || (matches!(a.mode, Mode::Write | Mode::Check)
                && a.paths.len() == 1
                && !a.paths[0].is_dir()))
    {
        die("--lines works on one file, or with --stdin");
    }
    if let Some(r) = &a.changed_since {
        if !matches!(
            a.mode,
            Mode::Write | Mode::Check | Mode::Lint | Mode::Syntax
        ) {
            die("--changed-since works with --check, --lint, --syntax and formatting in place");
        }
        // a revision, never an option git would act on
        if r.is_empty() || r.starts_with('-') {
            die(format!("--changed-since needs a git revision, not `{r}`"));
        }
    }
    if a.indent.is_some() && a.mode == Mode::SuggestStyle {
        die("--indent can't be combined with --suggest-style, which tries the indents itself");
    }
    if a.mode == Mode::Lsp && !a.paths.is_empty() {
        die("--lsp takes no paths (the editor sends the documents)");
    }
    if a.paths.is_empty() && !matches!(a.mode, Mode::Stdin | Mode::Lsp) {
        a.paths.push(PathBuf::from("."));
    }
    a
}

/// The config governing each file to process.
type Configs = HashMap<PathBuf, Arc<Config>>;

fn resolver(a: &Args) -> Resolver {
    let fixed = match a.mode {
        // the parse-only diagnostics never read a config
        Mode::Errors | Mode::Tree => Some(Config::default()),
        _ => a
            .config
            .as_ref()
            .map(|p| Config::load(p).unwrap_or_else(|e| die(e))),
    };
    let mut res = Resolver::new(fixed, a.indent);
    // the development reports keep using the working directory's config for every file
    if matches!(a.mode, Mode::Stats | Mode::LintStats) {
        let cwd = std::env::current_dir().unwrap_or_default();
        let cfg = res.dir(&cwd).unwrap_or_else(|e| die(e));
        res = Resolver::new(Some((*cfg).clone()), None);
    }
    res
}

/// `-v`: one line per config in play, in first-use order.
fn report<'a>(cfgs: impl IntoIterator<Item = &'a Config>) {
    let mut seen = HashSet::new();
    for c in cfgs {
        if seen.insert(&c.file) {
            match &c.file {
                Some(p) => eprintln!("config: {}", shown(p)),
                None => eprintln!("config: none found, using defaults"),
            }
        }
    }
}

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
        top || !groovyfmt::is_skipped_dir(p)
    } else {
        meta.is_file() && (top || groovyfmt::is_groovy_file(p))
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
fn collect(
    paths: &[PathBuf],
    res: &mut Resolver,
    write: bool,
) -> (Vec<PathBuf>, HashMap<PathBuf, PathBuf>, Configs) {
    let mut files = Vec::new();
    let mut links = HashMap::new();
    let cwd = std::fs::canonicalize(".").ok();
    for p in paths {
        if !p.exists() {
            die(format!("{}: no such file or directory", p.display()));
        }
        // components() drops a trailing `/` or `/.`, which would make the link resolve
        let last: PathBuf = p.components().collect();
        if write && last.is_symlink() {
            let real =
                std::fs::canonicalize(p).unwrap_or_else(|e| die(format!("{}: {e}", p.display())));
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
                Err(e) => die(format!("{}: {e}", p.display())),
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
fn par_map<T: Send>(
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

const NOT_UTF8: &str = "not valid UTF-8";

/// File contents, or `Ok(Err(reason))` for a file that is skipped rather than failing the
/// run: not UTF-8, not a regular file, or larger than `MAX_FILE_BYTES`. The checks run on
/// the opened handle, so a file swapped for something else after the walk is still caught.
fn read(path: &Path) -> Result<Result<String, &'static str>, String> {
    read_as(path, path).map(|r| r.map(|(src, _)| src))
}

/// `read`, naming the file `name` in errors, with the identity of the file read so a write
/// can check it is still the same file.
fn read_as(path: &Path, name: &Path) -> Result<Result<(String, FileId), &'static str>, String> {
    let err = |e: std::io::Error| format!("{}: {e}", shown(name));
    let f = std::fs::File::open(path).map_err(err)?;
    let meta = f.metadata().map_err(err)?;
    if !meta.is_file() {
        return Ok(Err("not a regular file"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Ok(Err("larger than 10 MB"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    f.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(Err("larger than 10 MB"));
    }
    Ok(String::from_utf8(bytes)
        .map(|src| (src, file_id(&meta)))
        .map_err(|_| NOT_UTF8))
}

/// Device and inode on unix; elsewhere there is nothing to compare.
type FileId = (u64, u64);

#[cfg(unix)]
fn file_id(m: &std::fs::Metadata) -> FileId {
    use std::os::unix::fs::MetadataExt;
    (m.dev(), m.ino())
}

#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> FileId {
    (0, 0)
}

fn changed() -> std::io::Error {
    std::io::Error::other("file changed while formatting")
}

static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Replace a file's contents atomically: write a temporary file beside it with the same
/// permissions and owner, then rename it over the path. A crash can't leave a truncated
/// file. A file with other hard links, or whose owner can't be kept, is written in place
/// instead. `read` is the identity of the file the contents came from; if the path no
/// longer names that regular file (swapped for a symlink or another file), nothing is
/// written.
fn write_atomic(path: &Path, contents: &str, read: FileId) -> std::io::Result<()> {
    write_keeping_owner(path, contents, read, keep_owner)
}

fn write_keeping_owner(
    path: &Path,
    contents: &str,
    read: FileId,
    keep_owner: impl Fn(&std::fs::File, &std::fs::Metadata) -> std::io::Result<()>,
) -> std::io::Result<()> {
    // This narrows the gap between the check and the rename below but can't close it: std
    // has no rename relative to a pinned file. The in-place path checks its open handle.
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || file_id(&meta) != read {
        return Err(changed());
    }
    let perms = meta.permissions();
    // a rename would replace a read-only file too; read-only means hands off
    if perms.readonly() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "file is read-only",
        ));
    }
    // a rename would leave the other names pointing at the old contents
    #[cfg(unix)]
    if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
        return write_in_place(path, read, contents);
    }
    // unique per call, so two workers given one file under different spellings can't collide;
    // short and not built from the file's name, which may already be as long as a name can be
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(tmp_name(seq));
    // a file already at that name isn't ours: fail without removing it
    let mut f = create_tmp(&tmp)?;
    // the new file belongs to whoever runs us (root, in a container); give it back
    if keep_owner(&f, &meta).is_err() {
        drop(f);
        let _ = std::fs::remove_file(&tmp);
        return write_in_place(path, read, contents);
    }
    let result = (|| {
        f.write_all(contents.as_bytes())?;
        f.set_permissions(perms)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn tmp_name(seq: usize) -> String {
    format!(".groovyfmt-{}-{seq}", std::process::id())
}

/// A new file at `tmp`, failing if one exists. Owner-only until the original permissions
/// are applied, so the contents of a private file are never readable by others in between.
fn create_tmp(tmp: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(tmp)
}

/// Give the new file `orig`'s owner and group. Only root can give a file away, so this
/// fails for most users when the owner differs.
#[cfg(unix)]
fn keep_owner(f: &std::fs::File, orig: &std::fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let new = f.metadata()?;
    if (new.uid(), new.gid()) == (orig.uid(), orig.gid()) {
        return Ok(());
    }
    std::os::unix::fs::fchown(f, Some(orig.uid()), Some(orig.gid()))
}

#[cfg(not(unix))]
fn keep_owner(_: &std::fs::File, _: &std::fs::Metadata) -> std::io::Result<()> {
    Ok(())
}

/// Overwrite the existing file. Not atomic: a crash mid-write can leave it part new, part
/// old. Taken only when a rename would split the file's hard links or hand it to another
/// owner (root formatting a developer's checkout), which is worse than that window.
fn write_in_place(path: &Path, read: FileId, contents: &str) -> std::io::Result<()> {
    // No truncation on open: nothing changes until the open handle is known to be the
    // regular file that was read, so a path swapped for a symlink or another file since is
    // not written. This compares identities instead of passing O_NOFOLLOW, whose value
    // differs by platform, isn't in std, and would need the libc crate. Without it (or
    // O_NONBLOCK), a path swapped for a FIFO makes the open wait for a reader; like the
    // read side, that needs a concurrent local attacker (the first audit's accepted risk).
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    let now = f.metadata()?;
    if !now.is_file() || file_id(&now) != read {
        return Err(changed());
    }
    f.set_len(0)?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()
}

/// Control characters escaped, so a crafted file name or file content (lint messages quote
/// source text) can't break a report line or inject CI workflow commands such as `::set-env`.
/// The GitHub Actions runner also takes the older `##[command]` form anywhere in a line, so
/// `##[` becomes `##\[`; nothing printed after this (`gh_escape`, `shown`) adds or removes
/// a `\` or `[`, so the sequence can't come back.
fn printable(s: &str) -> String {
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

/// A path for display. A leading `:` gets `./` in front, so no report line can start
/// with `::` and be taken as a GitHub Actions workflow command.
fn shown(p: &Path) -> String {
    let s = printable(&p.display().to_string());
    // after any leading whitespace too, in case the log reader trims it
    if s.trim_start().starts_with(':') {
        format!("./{s}")
    } else {
        s
    }
}

/// GitHub Actions workflow-command escaping for property values and messages.
fn gh_escape(s: &str, property: bool) -> String {
    let s = s
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A");
    if property {
        s.replace(':', "%3A").replace(',', "%2C")
    } else {
        s
    }
}

/// Where a file failed to parse: the first places, and how many there are in all.
type Diag = (Vec<SyntaxError>, usize);

/// `diag` for a skipped file whose reasons are listed (`-v`, `--strict`), else nothing.
fn diag_if(listed: bool, src: &str) -> Diag {
    if listed {
        syntax::check(src)
    } else {
        Diag::default()
    }
}

/// One line per place a file failed to parse, and a count of any not shown. To stdout for
/// `--syntax`, else to stderr below the file's `skipped` line.
fn report_syntax(name: &str, (shown_errs, total): &Diag, github: bool, out: fn(&str)) {
    for e in shown_errs {
        let message = printable(&e.message);
        out(&if github {
            format!(
                "::warning file={},line={},col={},title=could not parse::{}",
                gh_escape(name, true),
                e.line,
                e.col,
                gh_escape(&message, false)
            )
        } else {
            format!("{name}:{}:{}: could not parse: {message}", e.line, e.col)
        });
    }
    let more = total - shown_errs.len();
    if more > 0 {
        out(&if github {
            format!(
                "::warning file={},title=could not parse::{more} more places not shown",
                gh_escape(name, true)
            )
        } else {
            format!("{name}: {more} more places groovyfmt could not parse")
        });
    }
}

fn to_stderr(line: &str) {
    eprintln!("{line}");
}

/// Text to stdout as it is, errors ignored as in `say`.
fn print_raw(s: &str) {
    let _ = std::io::stdout().lock().write_all(s.as_bytes());
}

/// `format`, or with `--lines` only the lines in those ranges.
fn format_some(src: &str, style: &Style, lines: &[(usize, usize)]) -> Outcome {
    if lines.is_empty() {
        format(src, style)
    } else {
        diff::format_lines(src, style, lines)
    }
}

/// `--diff`: the patch turning `old` into `new`, or a `#` line saying why it isn't shown.
/// A patch has to print the file name and content as they are, so it is withheld when
/// either could act on a terminal, a CI log or the patch itself:
/// - the name goes into the `---` / `+++` headers before anything else is looked at: any
///   control character there (a line break would start a header of its own, naming
///   another file), or a `"` or `\`, which git would quote, withholds it;
/// - the patch is then scanned as a whole, not line by line (splitting into lines would
///   hide a CR): no control character but a tab, a line feed, and a CR right before a line
///   feed (CR LF files diff as they are; that CR ends a line, so nothing can follow it on
///   that line); no `##[` anywhere; no line starting `::` after leading whitespace.
///
/// `git apply` and `patch` skip a `#` line between files, so the rest still applies.
fn patch_for(path: &Path, old: &str, new: &str) -> String {
    let name = path.display().to_string();
    let withheld = || {
        format!(
            "# groovyfmt: {}: diff not shown, as the file name or a changed line holds a control character or a CI workflow command\n",
            shown(path)
        )
    };
    if name
        .chars()
        .any(|c| c.is_control() || c == '"' || c == '\\')
    {
        return withheld();
    }
    let patch = diff::unified(&name, old, new);
    let mut chars = patch.chars().peekable();
    while let Some(c) = chars.next() {
        let crlf = c == '\r' && chars.peek() == Some(&'\n');
        if c.is_control() && !matches!(c, '\t' | '\n') && !crlf {
            return withheld();
        }
    }
    if patch.contains("##[") || patch.split('\n').any(|l| l.trim_start().starts_with("::")) {
        withheld()
    } else {
        patch
    }
}

/// `--changed-since`: the files (canonical paths) that differ from git revision `rev`, in
/// the working tree or the index, plus untracked files that aren't ignored, under the
/// working directory. `rev` has been checked not to start with `-`.
fn git_changed(rev: &str) -> HashSet<PathBuf> {
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

/// The reason given for a file that doesn't parse.
const PARSE_ERROR: &str = "parse error";

fn parse_diagnostics((errs, _): Diag) -> Vec<Diagnostic> {
    errs.into_iter()
        .map(|e| Diagnostic {
            rule: report::PARSE_RULE,
            message: e.message,
            line: e.line,
            col: e.col,
            end_line: e.end_line,
            end_col: e.end_col,
            fixable: false,
        })
        .collect()
}

fn lint_diagnostic(v: lint::Violation) -> Diagnostic {
    Diagnostic {
        rule: v.rule,
        message: v.message,
        line: v.line,
        col: v.col,
        end_line: v.end_line,
        end_col: v.end_col,
        fixable: v.fix.is_some(),
    }
}

/// Where a run's results go. As text, each mode prints its own lines as it goes and the
/// summary line here; with `--output-format json` or `sarif`, the files are collected and
/// printed as one document on stdout instead, with no summary line (errors still go to
/// stderr).
struct Out {
    machine: Option<Machine>,
    files: Vec<FileReport>,
    /// More counts for the JSON summary
    extra: Vec<(&'static str, usize)>,
}

impl Out {
    fn new(a: &Args) -> Out {
        Out {
            machine: a.machine,
            files: Vec::new(),
            extra: Vec::new(),
        }
    }

    fn text(&self) -> bool {
        self.machine.is_none()
    }

    fn file(
        &mut self,
        path: &Path,
        status: &'static str,
        reason: Option<&str>,
        diagnostics: Vec<Diagnostic>,
    ) {
        if !self.text() {
            self.files.push(FileReport {
                path: path.display().to_string(),
                status,
                reason: reason.map(String::from),
                diagnostics,
            });
        }
    }

    /// `e` names the file first (`read` errors) or not (write errors); the record doesn't.
    fn error(&mut self, path: &Path, e: &str) {
        let e = e.strip_prefix(&format!("{}: ", shown(path))).unwrap_or(e);
        self.file(path, "error", Some(e), Vec::new());
    }

    fn summary(self, line: &str) {
        let doc = match self.machine {
            None => return eprintln!("{line}"),
            Some(Machine::Json) => report::json_with(&self.files, &self.extra),
            Some(Machine::Sarif) => report::sarif(&self.files, env!("CARGO_PKG_VERSION")),
        };
        let _ = std::io::stdout().lock().write_all(doc.as_bytes());
    }
}

fn main() {
    // a panic message can quote source text (a bad string slice prints the string), and
    // par_map turns the panic into an error but the hook still prints it first
    std::panic::set_hook(Box::new(|info| {
        eprintln!("groovyfmt: {}", printable(&info.to_string()));
    }));
    let a = parse_args();
    let mut res = resolver(&a);
    let code = match a.mode {
        Mode::Rules => rules(&one_config(&mut res, &a)),
        Mode::Explain(ref rule) => explain(rule, &one_config(&mut res, &a)),
        Mode::Stdin => stdin(&mut res, &a),
        // the --config file, read once; any other config is read afresh for each request
        Mode::Lsp => groovyfmt::lsp::run(
            a.config
                .as_ref()
                .map(|p| Config::load(p).unwrap_or_else(|e| die(e))),
            a.indent,
        ),
        _ => {
            // only write mode and --fix rewrite; reading through a named link is reading its target
            let write = a.mode == Mode::Write || (a.fix && !a.diff);
            let (mut files, links, cfgs) = collect(&a.paths, &mut res, write);
            if let Some(rev) = &a.changed_since {
                let changed = git_changed(rev);
                files.retain(|f| std::fs::canonicalize(f).is_ok_and(|c| changed.contains(&c)));
            }
            if a.verbose && !matches!(a.mode, Mode::Errors | Mode::Tree) {
                report(files.iter().map(|f| &*cfgs[f]));
            }
            match a.mode {
                Mode::Lint => {
                    // where --write-baseline writes: its own value, else --baseline, else the
                    // working directory's config, else the default name
                    let writing = a.write_baseline.as_ref().map(|w| {
                        w.clone()
                            .or_else(|| a.baseline.clone().filter(|b| !b.as_os_str().is_empty()))
                            .or_else(|| one_config(&mut res, &a).baseline.clone())
                            .unwrap_or_else(|| PathBuf::from(baseline::DEFAULT_FILE))
                    });
                    lint_files(&files, &links, &cfgs, &a, false, writing.as_deref())
                }
                Mode::Syntax => syntax(&files, &a),
                Mode::LintStats => lint_files(&files, &links, &cfgs, &a, true, None),
                Mode::Stats => stats(&files, &cfgs),
                Mode::SuggestStyle => suggest_style(&files, &cfgs),
                Mode::Errors => errors(&files),
                Mode::Tree => tree(&files),
                _ => fmt(&files, &links, &cfgs, &a),
            }
        }
    };
    std::process::exit(if WALK_FAILED.load(Ordering::Relaxed) {
        2
    } else {
        code
    });
}

/// `--explain`: one rule in full, from the rule table.
fn explain(name: &str, cfg: &Config) -> i32 {
    let Some(r) = RULES.iter().find(|r| r.name.eq_ignore_ascii_case(name)) else {
        let known: Vec<_> = RULES.iter().map(|r| r.name).collect();
        die(format!(
            "unknown rule `{name}` (known: {})",
            known.join(", ")
        ))
    };
    let indent = |code: &str| {
        code.lines()
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let here = if cfg.rule_enabled(r.name) {
        "on"
    } else {
        "off"
    };
    let fix = if r.fixable {
        "`groovyfmt --lint --fix` fixes it."
    } else {
        "Not fixed automatically."
    };
    let (other, setting) = if cfg.rule_enabled(r.name) {
        ("Turn it off", "false")
    } else {
        ("Turn it on", "true")
    };
    print_raw(&format!(
        "{name}: {description}\n\nOn by default: {d}. Here: {here}. {fix}\n\nWhy: {why}\n\nFlagged:\n{bad}\n\nInstead:\n{good}\n\n{other} in .groovyfmt.toml:\n    [lint]\n    {name} = {setting}\n",
        name = r.name,
        description = r.description,
        d = if r.default { "yes" } else { "no" },
        why = r.why,
        bad = indent(r.bad),
        good = indent(r.good),
    ));
    0
}

fn rules(cfg: &Config) -> i32 {
    for r in RULES {
        let on = if cfg.rule_enabled(r.name) {
            "on "
        } else {
            "off"
        };
        say(&format!("{on}  {:<22} {}", r.name, r.description));
    }
    0
}

/// The config for `--rules` and `--stdin`: the one formatting the file at `--stdin-filepath`
/// from disk would use, else the working directory's.
fn one_config(res: &mut Resolver, a: &Args) -> Arc<Config> {
    let cfg = match &a.stdin_filepath {
        Some(p) => res.of(p),
        None => res.dir(&std::env::current_dir().unwrap_or_default()),
    }
    .unwrap_or_else(|e| die(e));
    if a.verbose {
        report([&*cfg]);
    }
    cfg
}

fn stdin(res: &mut Resolver, a: &Args) -> i32 {
    let mut src = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut src) {
        die(format!("reading stdin: {e}"));
    }
    let path = a.stdin_filepath.as_deref();
    // excluded: hand the buffer back byte for byte, so editors and hooks see no change
    if path.is_some_and(|p| res.excluded(p).unwrap_or_else(|e| die(e))) {
        return std::io::stdout().write_all(&src).map_or(2, |_| 0);
    }
    let cfg = one_config(res, a);
    let src = String::from_utf8(src)
        .unwrap_or_else(|_| die("reading stdin: stream did not contain valid UTF-8"));
    let out = match format_some(&src, &cfg.style, &a.lines) {
        Outcome::Unchanged => src,
        Outcome::Formatted(out) => out,
        Outcome::Skipped(why) => {
            let name = path.map_or_else(|| "<stdin>".to_string(), shown);
            if a.github {
                eprintln!(
                    "::error file={}::Not formatted: {}",
                    gh_escape(&name, true),
                    gh_escape(why, false)
                );
            } else {
                eprintln!("groovyfmt: {name}: not formatted: {why}");
            }
            report_syntax(&name, &syntax::check(&src), a.github, to_stderr);
            std::process::exit(2);
        }
    };
    std::io::stdout().write_all(out.as_bytes()).map_or(2, |_| 0)
}

fn fmt(files: &[PathBuf], links: &HashMap<PathBuf, PathBuf>, cfgs: &Configs, a: &Args) -> i32 {
    let t = Instant::now();
    let check = a.mode == Mode::Check;
    let mut out = Out::new(a);
    let listed = (a.verbose || a.strict) && out.text();
    let want_diag = listed || !out.text();
    // Each worker writes (or, for --check, drops) the new text itself, so no more than one
    // output per thread is alive at once; only the small per-file results are kept, and
    // they're reported below in input order.
    let results = par_map(files, |p| {
        let real = links.get(p).map_or(p, PathBuf::as_path);
        let (src, id) = match read_as(real, p)? {
            Ok(read) => read,
            Err(why) => return Ok(Err((why, Diag::default()))),
        };
        Ok(match format_some(&src, &cfgs[p].style, &a.lines) {
            Outcome::Unchanged => Ok(None),
            Outcome::Skipped(why) => Err((why, diag_if(want_diag, &src))),
            Outcome::Formatted(out) if check => {
                Ok(Some((Ok(()), a.diff.then(|| patch_for(p, &src, &out)))))
            }
            Outcome::Formatted(out) => Ok(Some((write_atomic(real, &out, id), None))),
        })
    });
    let (mut changed, mut skipped, mut failed) = (0usize, 0usize, false);
    for (path, r) in files.iter().zip(results) {
        match r {
            Err(e) => {
                eprintln!("groovyfmt: {e}");
                failed = true;
                out.error(path, &e);
            }
            Ok(Ok(None)) => out.file(path, "unchanged", None, Vec::new()),
            Ok(Err((why, diag))) => {
                skipped += 1;
                if listed {
                    eprintln!("skipped {}: {why}", shown(path));
                    report_syntax(&shown(path), &diag, false, to_stderr);
                }
                out.file(path, "skipped", Some(why), parse_diagnostics(diag));
            }
            Ok(Ok(Some((written, patch)))) => {
                changed += 1;
                if let Err(e) = written {
                    eprintln!("groovyfmt: {}: {e}", shown(path));
                    failed = true;
                    out.error(path, &e.to_string());
                } else if let Some(patch) = patch {
                    print_raw(&patch);
                } else if !out.text() {
                    let status = if check {
                        "would_reformat"
                    } else {
                        "reformatted"
                    };
                    out.file(path, status, None, Vec::new());
                } else if check && a.github {
                    say(&format!(
                        "::error file={}::File is not formatted. Run groovyfmt to fix.",
                        gh_escape(&shown(path), true)
                    ));
                } else if check {
                    say(&shown(path));
                }
            }
        }
    }
    let verb = if check {
        "would reformat"
    } else {
        "reformatted"
    };
    let mut summary = format!("{verb} {changed} of {} files", files.len());
    if skipped > 0 {
        summary += &format!(", {skipped} skipped (-v shows why)");
    }
    if a.verbose {
        summary += &format!(" in {:.2}s", t.elapsed().as_secs_f64());
    }
    out.summary(&summary);
    if failed {
        2
    } else {
        i32::from((check && changed > 0) || (a.strict && skipped > 0))
    }
}

/// One file's lint result: the violations (after any fix), and with `--fix` what was fixed,
/// why a fix was refused, and whether the write failed.
struct Linted {
    violations: Vec<lint::Violation>,
    fixed: Vec<lint::Violation>,
    refused: Option<&'static str>,
    written: std::io::Result<()>,
    /// `--lint --fix --diff`: the patch, instead of writing
    patch: Option<String>,
    /// Violations the baseline held back, and its entries for this file that matched nothing
    held: usize,
    stale: usize,
    /// `--write-baseline`: the file's baseline path and its violations' keys
    keys: Option<(String, Vec<(String, String, usize)>)>,
}

/// The baseline file for a file governed by `cfg`: `--baseline` (`''` for none), else the
/// config's.
fn baseline_file(a: &Args, cfg: &Config) -> Option<PathBuf> {
    match &a.baseline {
        Some(f) if f.as_os_str().is_empty() => None,
        Some(f) => Some(f.clone()),
        None => cfg.baseline.clone(),
    }
}

/// The text of line `n` (1-based) of `text`.
fn line_of(text: &str, n: usize) -> &str {
    text.split('\n').nth(n.saturating_sub(1)).unwrap_or("")
}

fn lint_files(
    files: &[PathBuf],
    links: &HashMap<PathBuf, PathBuf>,
    cfgs: &Configs,
    a: &Args,
    stats_only: bool,
    writing: Option<&Path>,
) -> i32 {
    let t = Instant::now();
    let mut out = Out::new(a);
    // each file's baseline and its path in it; each baseline file is read once, and one that
    // can't be read or isn't a valid baseline is an error
    let mut loaded: HashMap<PathBuf, Arc<Baseline>> = HashMap::new();
    let mut load = |f: &Path| {
        loaded
            .entry(f.to_path_buf())
            .or_insert_with(|| Arc::new(Baseline::load(f).unwrap_or_else(|e| die(e))))
            .clone()
    };
    let dir = |f: &Path| f.parent().map(Path::to_path_buf).unwrap_or_default();
    let bases: HashMap<&Path, (Arc<Baseline>, String)> = match writing {
        // writing: the whole of every file is recorded, relative to the file written
        Some(target) => files
            .iter()
            .map(|p| {
                let rel = baseline::relative(p, &dir(target));
                (p.as_path(), (Arc::new(Baseline::default()), rel))
            })
            .collect(),
        None => files
            .iter()
            .filter_map(|p| {
                let f = baseline_file(a, &cfgs[p])?;
                Some((p.as_path(), (load(&f), baseline::relative(p, &dir(&f)))))
            })
            .collect(),
    };
    let listed = (a.verbose || a.strict) && out.text();
    let want_diag = listed || !out.text();
    let results = par_map(files, |p| {
        // with --fix, as when formatting: a named link's target is read and rewritten
        let real = links.get(p).map_or(p, PathBuf::as_path);
        let (src, id) = match read_as(real, p)? {
            Ok(read) => read,
            Err(why) => return Ok(Err((why, Diag::default()))),
        };
        let violations = match lint::lint_file(p, &src) {
            Ok(vs) => vs,
            Err(why) => return Ok(Err((why, diag_if(want_diag, &src)))),
        };
        let mut r = Linted {
            violations,
            fixed: Vec::new(),
            refused: None,
            written: Ok(()),
            patch: None,
            held: 0,
            stale: 0,
            keys: None,
        };
        let mut new_text: Option<String> = None;
        if a.fix {
            let cfg = &cfgs[p];
            // a violation the baseline has on a line like this one is left alone
            let base = bases.get(p.as_path());
            let wanted = |v: &lint::Violation, text: &str| {
                cfg.rule_enabled(v.rule)
                    && !base.is_some_and(|(b, rel)| b.covers_line(rel, v.rule, line_of(text, v.line)))
            };
            match fix::fix(p, &src, wanted) {
                Fix::Nothing => {}
                Fix::Refused(why) => r.refused = Some(why),
                Fix::Fixed(text, fixed) => {
                    if a.diff {
                        r.patch = Some(patch_for(p, &src, &text));
                    } else {
                        r.written = write_atomic(real, &text, id);
                    }
                    if r.written.is_ok() {
                        // the fix checked that the fixed text lints
                        r.violations = lint::lint_file(p, &text).unwrap_or_default();
                        r.fixed = fixed;
                        new_text = Some(text);
                    }
                }
            }
        }
        let text = new_text.as_deref().unwrap_or(&src);
        let cfg = &cfgs[p];
        r.violations.retain(|v| cfg.rule_enabled(v.rule));
        if let Some((b, rel)) = bases.get(p.as_path()) {
            if writing.is_some() {
                r.keys = Some((rel.clone(), baseline::keys(text, &r.violations)));
            } else {
                let (kept, held, stale) = b.filter(rel, text, std::mem::take(&mut r.violations));
                (r.violations, r.held, r.stale) = (kept, held, stale);
            }
        }
        Ok(Ok(r))
    });
    let (mut skipped, mut total, mut failed) = (0usize, 0usize, false);
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut fixed_files, mut fixed_by_rule) = (0usize, BTreeMap::<&str, usize>::new());
    let (mut held, mut stale) = (0usize, 0usize);
    // --write-baseline: the entries, and the files they cover
    let (mut entries, mut recorded) = (Vec::new(), std::collections::BTreeSet::new());
    for (path, r) in files.iter().zip(results) {
        match r {
            Err(e) => {
                eprintln!("groovyfmt: {e}");
                failed = true;
                out.error(path, &e);
            }
            Ok(Err((why, diag))) => {
                skipped += 1;
                if listed {
                    eprintln!("skipped {}: {why}", shown(path));
                    report_syntax(&shown(path), &diag, false, to_stderr);
                }
                out.file(path, "skipped", Some(why), parse_diagnostics(diag));
            }
            Ok(Ok(r)) => {
                if let Err(e) = &r.written {
                    eprintln!("groovyfmt: {}: {e}", shown(path));
                    failed = true;
                    out.error(path, &e.to_string());
                    continue;
                }
                fixed_files += usize::from(!r.fixed.is_empty());
                (held, stale) = (held + r.held, stale + r.stale);
                if let Some((rel, keys)) = r.keys {
                    entries.extend(keys.into_iter().map(|(rule, fp, i)| (rel.clone(), rule, fp, i)));
                    recorded.insert(rel);
                    continue;
                }
                for v in &r.fixed {
                    *fixed_by_rule.entry(v.rule).or_default() += 1;
                }
                let refused = r.refused.map(|why| format!("not fixed: {why}"));
                if let Some(why) = &refused
                    && out.text()
                {
                    eprintln!("{} {why}", shown(path));
                }
                if let Some(patch) = &r.patch {
                    print_raw(patch);
                }
                let cfg = &cfgs[path];
                let vs: Vec<_> = r
                    .violations
                    .into_iter()
                    .filter(|v| cfg.rule_enabled(v.rule))
                    .collect();
                total += vs.len();
                for v in &vs {
                    *by_rule.entry(v.rule).or_default() += 1;
                }
                if !out.text() {
                    let status = if r.fixed.is_empty() {
                        "unchanged"
                    } else {
                        "fixed"
                    };
                    let diagnostics = vs.into_iter().map(lint_diagnostic).collect();
                    out.file(path, status, refused.as_deref(), diagnostics);
                    continue;
                }
                for v in vs {
                    // with --diff, stdout is the patch alone
                    if stats_only || a.diff {
                        continue;
                    }
                    let message = printable(&v.message);
                    say(&if a.github {
                        format!(
                            "::warning file={},line={},col={},title={}::{}",
                            gh_escape(&shown(path), true),
                            v.line,
                            v.col,
                            v.rule,
                            gh_escape(&message, false)
                        )
                    } else {
                        let fixable = if v.fix.is_some() { " (fixable)" } else { "" };
                        format!(
                            "{}:{}:{}: {}: {message}{fixable}",
                            shown(path),
                            v.line,
                            v.col,
                            v.rule
                        )
                    });
                }
            }
        }
    }
    if a.fix && out.text() {
        let n: usize = fixed_by_rule.values().sum();
        let parts: Vec<_> = fixed_by_rule
            .iter()
            .map(|(r, n)| format!("{r} {n}"))
            .collect();
        let rules = if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join(", "))
        };
        let verb = if a.diff { "would fix" } else { "fixed" };
        eprintln!("{verb} {n} violations in {fixed_files} files{rules}");
    }
    if let Some(target) = writing {
        // entries for files this run didn't lint (or couldn't parse) are kept
        let old = if target.exists() {
            Baseline::load(target).unwrap_or_else(|e| die(e))
        } else {
            Baseline::default()
        };
        let n = entries.len();
        let doc = baseline::write(entries, &old, &recorded);
        if let Err(e) = write_new(target, &doc) {
            eprintln!("groovyfmt: {}: {e}", shown(target));
            return 2;
        }
        eprintln!(
            "wrote {n} violations in {} files to {}",
            recorded.len(),
            shown(target)
        );
        return if failed {
            2
        } else {
            i32::from(a.strict && skipped > 0)
        };
    }
    let mut summary = format!("{total} violations in {} files", files.len());
    if !by_rule.is_empty() {
        let parts: Vec<_> = by_rule.iter().map(|(r, n)| format!("{r} {n}")).collect();
        summary += &format!(" ({})", parts.join(", "));
    }
    if held > 0 {
        summary += &format!(", {held} in the baseline");
    }
    if stale > 0 {
        summary += &format!(", {stale} stale baseline entries (--write-baseline drops them)");
    }
    out.extra = vec![("baselined", held), ("stale_baseline_entries", stale)];
    if skipped > 0 {
        summary += &format!(", {skipped} skipped (-v shows why)");
    }
    if a.verbose || stats_only {
        summary += &format!(" in {:.2}s", t.elapsed().as_secs_f64());
    }
    out.summary(&summary);
    // with --diff nothing was written, so the fixable violations are still there
    let left = total
        + if a.diff {
            fixed_by_rule.values().sum()
        } else {
            0
        };
    if failed {
        2
    } else {
        // with --strict, a stale baseline fails too, so it can only shrink
        i32::from(left > 0 || (a.strict && (skipped > 0 || stale > 0)))
    }
}

/// Write `contents` to `path`, a new file or one replaced atomically, as formatting does.
fn write_new(path: &Path, contents: &str) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => write_atomic(path, contents, file_id(&m)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = path.with_file_name(tmp_name(seq));
            let mut f = create_tmp(&tmp)?;
            let result = (|| {
                f.write_all(contents.as_bytes())?;
                // a file to commit: readable like any other in the checkout
                #[cfg(unix)]
                f.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
                f.sync_all()?;
                std::fs::rename(&tmp, path)
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            result
        }
        Err(e) => Err(e),
    }
}

/// `--syntax`: parse only, and report every file that doesn't parse cleanly.
fn syntax(files: &[PathBuf], a: &Args) -> i32 {
    let t = Instant::now();
    let results = par_map(files, |p| read(p).map(|src| src.map(|s| syntax::check(&s))));
    let mut out = Out::new(a);
    let (mut bad, mut skipped, mut failed) = (0usize, 0usize, false);
    for (path, r) in files.iter().zip(results) {
        match r {
            Err(e) => {
                eprintln!("groovyfmt: {e}");
                failed = true;
                out.error(path, &e);
            }
            Ok(Err(why)) => {
                skipped += 1;
                if (a.verbose || a.strict) && out.text() {
                    eprintln!("skipped {}: {why}", shown(path));
                }
                out.file(path, "skipped", Some(why), Vec::new());
            }
            Ok(Ok((_, 0))) => out.file(path, "unchanged", None, Vec::new()),
            Ok(Ok(diag)) => {
                bad += 1;
                if out.text() {
                    report_syntax(&shown(path), &diag, a.github, say);
                }
                out.file(path, "skipped", Some(PARSE_ERROR), parse_diagnostics(diag));
            }
        }
    }
    let mut summary = format!("could not parse {bad} of {} files", files.len());
    if skipped > 0 {
        summary += &format!(", {skipped} skipped (-v shows why)");
    }
    if a.verbose {
        summary += &format!(" in {:.2}s", t.elapsed().as_secs_f64());
    }
    out.summary(&summary);
    if failed {
        2
    } else {
        i32::from(bad > 0 || (a.strict && skipped > 0))
    }
}

/// Development: corpus report with an idempotence probe.
fn stats(files: &[PathBuf], cfgs: &Configs) -> i32 {
    let t = Instant::now();
    let results = par_map(files, |p| {
        let src = match read(p)? {
            Ok(src) => src,
            Err(why) => return Ok((0, Outcome::Skipped(why), false, 0)),
        };
        let style = &cfgs[p].style;
        let out = format(&src, style);
        let nonidem =
            matches!(&out, Outcome::Formatted(o) if format(o, style) != Outcome::Unchanged);
        let lines = match &out {
            Outcome::Formatted(o) => lines_changed(&src, o),
            _ => 0,
        };
        Ok((src.len(), out, nonidem, lines))
    });
    let (mut bytes, mut unchanged, mut changed, mut nonidem, mut lines, mut skipped) =
        (0, 0, 0, 0, 0, 0);
    for (path, r) in files.iter().zip(results) {
        let Ok((len, out, ni, l)) = r else {
            skipped += 1;
            continue;
        };
        bytes += len;
        lines += l;
        match out {
            Outcome::Unchanged => unchanged += 1,
            Outcome::Formatted(_) => changed += 1,
            Outcome::Skipped(_) => skipped += 1,
        }
        if ni {
            nonidem += 1;
            say(&format!("NONIDEMPOTENT {}", shown(path)));
        }
    }
    let secs = t.elapsed().as_secs_f64();
    say(&format!(
        "files={} unchanged={unchanged} formatted={changed} lines_changed={lines} nonidempotent={nonidem} skipped={skipped} MB={:.1} secs={secs:.2}",
        files.len(),
        bytes as f64 / 1e6
    ));
    i32::from(nonidem > 0)
}

/// Lines that differ between `src` and its formatted text `out`.
fn lines_changed(src: &str, out: &str) -> usize {
    src.lines().zip(out.lines()).filter(|(a, b)| a != b).count()
        + src.lines().count().abs_diff(out.lines().count())
}

/// `--suggest-style` candidates as (indent, continuation_indent, use_tabs), simplest first:
/// the defaults, then one key, then two. A tie on files unchanged goes to the one changing
/// fewer lines, then to the earlier one.
const CANDIDATES: [(usize, Option<usize>, bool); 6] = [
    (4, None, false),
    (2, None, false),
    (4, None, true),
    (4, Some(8), false),
    (2, Some(4), false),
    (4, Some(8), true),
];

/// The config lines for a candidate; none for the defaults.
fn style_lines((indent, continuation, tabs): (usize, Option<usize>, bool)) -> Vec<String> {
    let mut v = Vec::new();
    if indent != 4 {
        v.push(format!("indent = {indent}"));
    }
    if let Some(c) = continuation {
        v.push(format!("continuation_indent = {c}"));
    }
    if tabs {
        v.push("use_tabs = true".into());
    }
    v
}

/// Format every file under each candidate style in memory, keeping each file's other config
/// settings, and print the config lines for the one leaving the most files unchanged.
fn suggest_style(files: &[PathBuf], cfgs: &Configs) -> i32 {
    if files.is_empty() {
        die("no Groovy files found, nothing to suggest");
    }
    let t = Instant::now();
    // per file and candidate: `None` when skipped, else whether the file changes and how
    // many lines do (0 when only the final newline does)
    let results = par_map(files, |p| {
        let src = match read(p)? {
            Ok(src) => src,
            Err(_) => return Ok([None; CANDIDATES.len()]),
        };
        // ponytail: one full format per candidate, about 6x a --check run; parsing the
        // input once for all candidates would save roughly 40% if this ever matters
        Ok(CANDIDATES.map(|(indent, continuation, use_tabs)| {
            let style = Style {
                indent,
                continuation,
                use_tabs,
                ..cfgs[p].style
            };
            match format(&src, &style) {
                Outcome::Unchanged => Some((false, 0)),
                Outcome::Formatted(out) => Some((true, lines_changed(&src, &out))),
                Outcome::Skipped(_) => None,
            }
        }))
    });
    let mut tally = [(0usize, 0usize, 0usize); CANDIDATES.len()]; // unchanged, changed, lines
    let (mut measured, mut failed) = (0usize, false);
    for r in results {
        let per = r.unwrap_or_else(|e| {
            eprintln!("groovyfmt: {e}");
            failed = true;
            [None; CANDIDATES.len()]
        });
        measured += usize::from(per.iter().any(Option::is_some));
        for (c, l) in tally.iter_mut().zip(per) {
            match l {
                Some((false, _)) => c.0 += 1,
                Some((true, l)) => (c.1, c.2) = (c.1 + 1, c.2 + l),
                None => {}
            }
        }
    }
    if measured == 0 {
        die(format!(
            "none of the {} files could be formatted, nothing to suggest",
            files.len()
        ));
    }
    let best = (0..CANDIDATES.len())
        .max_by_key(|&i| {
            use std::cmp::Reverse;
            (tally[i].0, Reverse(tally[i].2), Reverse(i))
        })
        .unwrap_or(0);
    eprintln!("  {:<42} {:>7} {:>7}", "style", "files", "lines");
    for (i, (c, (_, changed, lines))) in CANDIDATES.iter().zip(tally).enumerate() {
        let name = style_lines(*c).join(", ");
        let name = if name.is_empty() { "defaults" } else { &name };
        let mark = if i == best { "*" } else { " " };
        eprintln!("{mark} {name:<42} {changed:>7} {lines:>7}");
    }
    eprintln!(
        "files: files changed, of {} ({} skipped under every style); lines: lines changed; in {:.2}s",
        files.len(),
        files.len() - measured,
        t.elapsed().as_secs_f64()
    );
    let lines = style_lines(CANDIDATES[best]);
    let unchanged = format!("{} of {} files unchanged", tally[best].0, files.len());
    if lines.is_empty() {
        say(&format!("# the defaults fit best: {unchanged}"));
    } else {
        say(&format!(
            "# suggested by groovyfmt --suggest-style: {unchanged}"
        ));
        for l in lines {
            say(&l);
        }
    }
    if failed { 2 } else { 0 }
}

/// Development: "<parent kind>\t<ERROR|MISSING x>\t<line>\t<path:line>" per file that fails to parse.
fn errors(files: &[PathBuf]) -> i32 {
    let rows = par_map(files, |p| {
        let Ok(src) = read(p)? else { return Ok(None) };
        let tree = groovyfmt::parse(&src);
        let Some(e) = groovyfmt::first_error(&tree) else {
            return Ok(None);
        };
        let line = src.lines().nth(e.start_position().row).unwrap_or("").trim();
        let parent = e.parent().map_or("-", |p| p.kind());
        let kind = if e.is_missing() {
            format!("MISSING {}", e.kind())
        } else {
            "ERROR".into()
        };
        Ok(Some(format!(
            "{parent}\t{kind}\t{}\t{}:{}",
            printable(&line.chars().take(100).collect::<String>()),
            shown(p),
            e.start_position().row + 1
        )))
    });
    for r in rows.into_iter().flatten().flatten() {
        say(&r);
    }
    0
}

/// Development: dump each file's parse tree.
fn tree(files: &[PathBuf]) -> i32 {
    for f in files {
        match read(f) {
            Ok(Ok(src)) => say(&groovyfmt::parse(&src).root_node().to_sexp()),
            Ok(Err(why)) => eprintln!("groovyfmt: {}: {why}", shown(f)),
            Err(e) => eprintln!("groovyfmt: {e}"),
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held by tests that call `write_atomic`, so none takes the temp name another predicts.
    static TMP_NAMES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn id(p: &Path) -> FileId {
        file_id(&std::fs::metadata(p).unwrap())
    }

    #[test]
    fn printable_neutralizes_line_breaks() {
        assert_eq!(
            printable("a\n::set-env name=X::y\r"),
            "a\\n::set-env name=X::y\\r"
        );
        assert_eq!(printable("plain é"), "plain é");
    }

    #[test]
    fn write_atomic_never_removes_a_file_it_did_not_create() {
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-tmpname-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("A.groovy");
        std::fs::write(&path, "old\n").unwrap();
        // squat on the temp name the next call will pick
        let squat = d.join(tmp_name(TMP_SEQ.load(Ordering::Relaxed)));
        std::fs::write(&squat, "precious\n").unwrap();
        let err = write_atomic(&path, "new\n", id(&path)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&squat).unwrap(), "precious\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
        write_atomic(&path, "new\n", id(&path)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_atomic_handles_the_longest_file_names() {
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-longname-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // 255 bytes, NAME_MAX on Linux and macOS: a temp name built from it can't be created
        let path = d.join(format!("{}.groovy", "L".repeat(248)));
        std::fs::write(&path, "old\n").unwrap();
        let r = write_atomic(&path, "new\n", id(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        r.unwrap();
        assert_eq!(text, "new\n");
    }

    #[test]
    fn printable_neutralizes_legacy_workflow_commands() {
        assert_eq!(
            printable("x##[error]y ###[add-mask]z ##[##[w"),
            "x##\\[error]y ###\\[add-mask]z ##\\[##\\[w"
        );
        assert_eq!(printable("#\n#[x"), "#\\n#[x");
        for s in ["a##[b", "##[stop-commands]t%:,\r"] {
            let p = printable(s);
            for out in [
                shown(Path::new(s)),
                gh_escape(&p, true),
                gh_escape(&p, false),
            ] {
                assert!(!out.contains("##["), "{out}");
            }
        }
    }

    #[test]
    fn shown_never_starts_a_line_with_a_colon() {
        assert_eq!(
            shown(Path::new("::warning title=x::y.groovy")),
            "./::warning title=x::y.groovy"
        );
        assert_eq!(shown(Path::new("src/A.groovy")), "src/A.groovy");
        assert_eq!(shown(Path::new(" \u{a0}::x.groovy")), "./ \u{a0}::x.groovy");
    }

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

    #[cfg(unix)]
    #[test]
    fn temp_file_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("groovyfmt-tmpmode-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let tmp = d.join(".A.groovy.tmp");
        let _ = std::fs::remove_file(&tmp);
        let f = create_tmp(&tmp).unwrap();
        let mode = f.metadata().unwrap().permissions().mode() & 0o777;
        assert!(
            create_tmp(&tmp).is_err(),
            "an existing file is never reused"
        );
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(mode, 0o600, "{mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn rename_unless_hard_linked_or_owner_lost() {
        use std::os::unix::fs::MetadataExt;
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-inplace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("A.groovy"), d.join("B.groovy"));
        let ino = |p: &Path| std::fs::metadata(p).unwrap().ino();

        // the common case renames: a new inode
        std::fs::write(&a, "old\n").unwrap();
        let before = ino(&a);
        write_atomic(&a, "new\n", id(&a)).unwrap();
        assert_ne!(ino(&a), before, "renamed");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "new\n");

        // owner can't be kept: written in place, temp file removed
        let before = ino(&a);
        write_keeping_owner(&a, "longer than before\n", id(&a), |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap();
        assert_eq!(ino(&a), before, "in place");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "longer than before\n");
        write_keeping_owner(&a, "short\n", id(&a), |_, _| {
            Err(std::io::Error::other("x"))
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "short\n", "truncated");

        // only the file that was read is written, by either path
        let other = d.join("Other.groovy");
        std::fs::write(&other, "other\n").unwrap();
        assert!(write_atomic(&a, "evil\n", id(&other)).is_err(), "rename");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "short\n");

        // a second hard link: both names see the new text
        std::fs::hard_link(&a, &b).unwrap();
        write_atomic(&b, "linked\n", id(&b)).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "linked\n");
        assert_eq!((ino(&a), ino(&b)), (before, before));
        assert_eq!(std::fs::metadata(&a).unwrap().nlink(), 2);

        assert!(write_in_place(&a, id(&other), "evil\n").is_err());
        assert!(write_atomic(&a, "evil\n", id(&other)).is_err());
        assert!(write_atomic(&b, "evil\n", id(&other)).is_err(), "linked");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "linked\n");
        let was = id(&a);
        std::fs::remove_file(&a).unwrap();
        std::os::unix::fs::symlink(&other, &a).unwrap();
        assert!(write_in_place(&a, was, "evil\n").is_err());
        assert!(
            write_atomic(&a, "evil\n", was).is_err(),
            "not a regular file"
        );
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "other\n");
        assert!(std::fs::symlink_metadata(&a).unwrap().is_symlink());

        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".groovyfmt-"))
            .collect();
        let _ = std::fs::remove_dir_all(&d);
        assert!(leftovers.is_empty(), "no temp files left behind");
    }

    #[test]
    fn patches_are_withheld_for_unsafe_names_and_content() {
        const UGLY: &str = "class A {\ndef f() {}\n}\n";
        const PRETTY: &str = "class A {\n    def f() {}\n}\n";
        let withheld = |p: &str| patch_for(Path::new(p), UGLY, PRETTY).starts_with("# groovyfmt: ");
        // a name is checked before it reaches a header
        for name in [
            "x\n+++ b/other.groovy",
            "x\r\n+++ b/other.groovy",
            "a\rb.groovy",
            "a\tb.groovy",
            "a\x1b[31m.groovy",
            "a\u{85}b.groovy",
            "q\"x.groovy",
            "b\\s.groovy",
            "x##[error]y.groovy",
        ] {
            assert!(withheld(name), "{name:?}");
            let out = patch_for(Path::new(name), UGLY, PRETTY);
            assert_eq!(out.lines().count(), 1, "{name:?}: one notice line");
            assert!(
                out.starts_with("# groovyfmt: ") && !out.contains("##["),
                "{out:?}"
            );
        }
        // a leading `::` in a name lands after `--- a/`, never at a line start
        let p = patch_for(Path::new("::warning::x.groovy"), UGLY, PRETTY);
        assert!(p.starts_with("--- a/::warning::x.groovy\n"), "{p}");
        assert!(p.split('\n').all(|l| !l.trim_start().starts_with("::")));
        assert!(!withheld("src/A b-c.groovy"));

        // content: CR LF is kept as it is; a CR anywhere else, ESC, `##[` or a `::` line is not
        let crlf = patch_for(
            Path::new("A.groovy"),
            "class A {\r\ndef f() {}\r\n}\r\n",
            "class A {\r\n    def f() {}\r\n}\r\n",
        );
        assert!(
            crlf.contains("\n-def f() {}\r\n+    def f() {}\r\n"),
            "{crlf:?}"
        );
        for line in [
            "def s = 'a\rb'",
            "def s = '\x1b[2J'",
            "def s = '##[error]'",
            "// x\n::set-output x",
        ] {
            let old = format!("class A {{\n{line}\ndef f() {{}}\n}}\n");
            let new = format!("class A {{\n{line}\n    def f() {{}}\n}}\n");
            assert!(
                patch_for(Path::new("A.groovy"), &old, &new).starts_with("# groovyfmt: "),
                "{line:?}"
            );
        }
    }

    #[test]
    fn gh_escape_properties_and_messages() {
        assert_eq!(gh_escape("a:b,c%\r\n", true), "a%3Ab%2Cc%25%0D%0A");
        assert_eq!(gh_escape("a:b,c%\n", false), "a:b,c%25%0A");
    }
}
