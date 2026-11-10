use groovyfmt::config::{self, Config};
use groovyfmt::format::{Outcome, format};
use groovyfmt::lint::{self, RULES};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

const HELP: &str = "\
groovyfmt: formatter and linter for Groovy, Gradle build scripts, and Jenkinsfiles

Usage: groovyfmt [OPTIONS] [PATH]...

PATHs are files or directories (default: the current directory). Directories are
searched for *.groovy, *.gradle, *.gvy, *.gy, *.gsh and Jenkinsfile, skipping
hidden directories, build, out, target and node_modules, and anything matching
the config file's `exclude` patterns (which also apply to files named here).

Modes:
      (default)            Format files in place
      --check              List files that would be reformatted; exit 1 if any
      --lint               Report lint violations; exit 1 if any
      --stdin              Format stdin to stdout (for editors)
      --stdin-filepath <PATH>
                           With --stdin: find the config as if formatting PATH (which
                           need not exist), name it in messages, and if PATH is
                           excluded, echo stdin unchanged

Options:
      --config <FILE>      Use FILE instead of the nearest .groovyfmt.toml / groovyfmt.toml
      --indent <N>         Spaces per indent level (overrides the config file)
      --output-format <F>  text (default) or github (GitHub Actions annotations)
      --rules              List lint rules and whether each is on, then exit
  -v, --verbose            Also report skipped files and timing
  -h, --help               Print help
  -V, --version            Print version

Formatting changes whitespace only and never touches a file it can't parse, or
one whose tokens would change. Exit status: 0 clean, 1 files to format or lint
violations found, 2 error.
";

#[derive(Default)]
struct Args {
    mode: Mode,
    paths: Vec<PathBuf>,
    config: Option<PathBuf>,
    stdin_filepath: Option<PathBuf>,
    indent: Option<usize>,
    github: bool,
    verbose: bool,
}

#[derive(Default, PartialEq)]
enum Mode {
    #[default]
    Write,
    Check,
    Lint,
    Stdin,
    Rules,
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

/// Set when a directory couldn't be read during the walk; the run then exits 2.
static WALK_FAILED: AtomicBool = AtomicBool::new(false);

/// Files larger than this are skipped: generated or vendored code, and a guard against
/// a crafted input exhausting memory.
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

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
            _ => die("--check, --lint, --stdin and --rules can't be combined"),
        };
    };
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| {
            inline
                .clone()
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
            "--stdin" => set_mode(&mut a, Mode::Stdin),
            "--rules" => set_mode(&mut a, Mode::Rules),
            "--stats" => set_mode(&mut a, Mode::Stats),
            "--errors" => set_mode(&mut a, Mode::Errors),
            "--tree" => set_mode(&mut a, Mode::Tree),
            "-v" | "--verbose" => a.verbose = true,
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
                "text" => a.github = false,
                "github" => a.github = true,
                other => die(format!(
                    "unknown --output-format `{other}` (text or github)"
                )),
            },
            f if f.starts_with('-') && f != "-" => {
                die(format!("unknown option `{f}` (see --help)"))
            }
            _ => a.paths.push(PathBuf::from(arg)),
        }
    }
    if a.stdin_filepath.is_some() && a.mode != Mode::Stdin {
        die("--stdin-filepath needs --stdin");
    }
    if a.paths.is_empty() && a.mode != Mode::Stdin {
        a.paths.push(PathBuf::from("."));
    }
    a
}

fn load_config(a: &Args) -> Config {
    let cwd = std::env::current_dir().unwrap_or_default();
    // the config that formatting the file at --stdin-filepath from disk would see
    let start = match &a.stdin_filepath {
        Some(p) => config::normalize(&cwd.join(p))
            .parent()
            .map_or(cwd, Path::to_path_buf),
        None => cwd,
    };
    let path = a.config.clone().or_else(|| config::discover(&start));
    let mut cfg = match &path {
        Some(p) => Config::load(p).unwrap_or_else(|e| die(e)),
        None => Config::default(),
    };
    if let Some(n) = a.indent {
        cfg.style.indent = n;
    }
    if a.verbose {
        match &path {
            Some(p) => eprintln!("config: {}", shown(p)),
            None => eprintln!("config: none found, using defaults"),
        }
    }
    cfg
}

/// Paths named on the command line are taken as given (a named symlink is followed).
/// Below them, symlinks are never followed, so a link cycle can't loop the walk and a link
/// out of the tree can't make us read or rewrite files elsewhere; only regular files count.
fn walk(p: &Path, top: bool, out: &mut Vec<PathBuf>, cfg: &Config) {
    let meta = if top {
        std::fs::metadata(p)
    } else {
        std::fs::symlink_metadata(p)
    };
    let Ok(meta) = meta else { return };
    if cfg.excluded(p) {
        return;
    }
    if meta.is_dir() {
        if !top && groovyfmt::is_skipped_dir(p) {
            return;
        }
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
        for path in entries {
            walk(&path, false, out, cfg);
        }
    } else if meta.is_file() && (top || groovyfmt::is_groovy_file(p)) {
        // `./src/A.groovy` reads as `src/A.groovy`, as in git and cargo output
        out.push(p.strip_prefix(".").unwrap_or(p).to_path_buf());
    }
}

/// The files to process, named as the user typed them, and for each symlink to a file named
/// on the command line, the file it points to. That file is the one read and rewritten, so
/// the link stays a link; resolving it once here means a link retargeted later can't
/// redirect the write.
fn collect(paths: &[PathBuf], cfg: &Config) -> (Vec<PathBuf>, HashMap<PathBuf, PathBuf>) {
    let mut files = Vec::new();
    let mut links = HashMap::new();
    for p in paths {
        if !p.exists() {
            die(format!("{}: no such file or directory", p.display()));
        }
        let n = files.len();
        walk(p, true, &mut files, cfg);
        if files.len() > n && p.is_symlink() && p.is_file() {
            match std::fs::canonicalize(p) {
                // excluding the target excludes it under any name
                Ok(real) if cfg.excluded(&real) => files.truncate(n),
                Ok(real) => {
                    links.insert(files[n].clone(), real);
                }
                Err(e) => die(format!("{}: {e}", p.display())),
            }
        }
    }
    // overlapping arguments (`. src`, a file named twice or spelled differently): each file
    // once, first-seen order
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| seen.insert(std::fs::canonicalize(f).unwrap_or_else(|_| f.clone())));
    (files, links)
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
    let mut out: Vec<(usize, Result<T, String>)> = std::thread::scope(|s| {
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
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    out.sort_unstable_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
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
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    // unique per call, so two workers given one file under different spellings can't collide
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!(".{name}.groovyfmt-{}-{seq}", std::process::id()));
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

fn main() {
    // a panic message can quote source text (a bad string slice prints the string), and
    // par_map turns the panic into an error but the hook still prints it first
    std::panic::set_hook(Box::new(|info| {
        eprintln!("groovyfmt: {}", printable(&info.to_string()));
    }));
    let a = parse_args();
    // the walk needs `exclude`; the parse-only diagnostics never read a config
    let cfg = if matches!(a.mode, Mode::Errors | Mode::Tree) {
        Config::default()
    } else {
        load_config(&a)
    };
    let code = match a.mode {
        Mode::Rules => rules(&cfg),
        Mode::Stdin => stdin(&cfg, &a),
        Mode::Write | Mode::Check => {
            let (files, links) = collect(&a.paths, &cfg);
            fmt(&files, &links, &cfg, &a)
        }
        // these only read, and reading through a named link is reading its target
        Mode::Lint => lint_files(&collect(&a.paths, &cfg).0, &cfg, &a, false),
        Mode::LintStats => lint_files(&collect(&a.paths, &cfg).0, &cfg, &a, true),
        Mode::Stats => stats(&collect(&a.paths, &cfg).0, &cfg),
        Mode::Errors => errors(&collect(&a.paths, &cfg).0),
        Mode::Tree => tree(&collect(&a.paths, &cfg).0),
    };
    std::process::exit(if WALK_FAILED.load(Ordering::Relaxed) {
        2
    } else {
        code
    });
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

fn stdin(cfg: &Config, a: &Args) -> i32 {
    let mut src = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut src) {
        die(format!("reading stdin: {e}"));
    }
    let path = a.stdin_filepath.as_deref();
    // excluded: hand the buffer back byte for byte, so editors and hooks see no change
    if path.is_some_and(|p| cfg.excluded(p)) {
        return std::io::stdout().write_all(&src).map_or(2, |_| 0);
    }
    let src = String::from_utf8(src)
        .unwrap_or_else(|_| die("reading stdin: stream did not contain valid UTF-8"));
    let out = match format(&src, &cfg.style) {
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
                std::process::exit(2);
            }
            die(format!("{name}: not formatted: {why}"))
        }
    };
    std::io::stdout().write_all(out.as_bytes()).map_or(2, |_| 0)
}

fn fmt(files: &[PathBuf], links: &HashMap<PathBuf, PathBuf>, cfg: &Config, a: &Args) -> i32 {
    let t = Instant::now();
    let check = a.mode == Mode::Check;
    // Each worker writes (or, for --check, drops) the new text itself, so no more than one
    // output per thread is alive at once; only the small per-file results are kept, and
    // they're reported below in input order.
    let results = par_map(files, |p| {
        let real = links.get(p).map_or(p, PathBuf::as_path);
        let (src, id) = match read_as(real, p)? {
            Ok(read) => read,
            Err(why) => return Ok(Err(why)),
        };
        Ok(match format(&src, &cfg.style) {
            Outcome::Unchanged => Ok(None),
            Outcome::Skipped(why) => Err(why),
            Outcome::Formatted(_) if check => Ok(Some(Ok(()))),
            Outcome::Formatted(out) => Ok(Some(write_atomic(real, &out, id))),
        })
    });
    let (mut changed, mut skipped, mut failed) = (0usize, 0usize, false);
    for (path, r) in files.iter().zip(results) {
        match r {
            Err(e) => {
                eprintln!("groovyfmt: {e}");
                failed = true;
            }
            Ok(Ok(None)) => {}
            Ok(Err(why)) => {
                skipped += 1;
                if a.verbose {
                    eprintln!("skipped {}: {why}", shown(path));
                }
            }
            Ok(Ok(Some(written))) => {
                changed += 1;
                if let Err(e) = written {
                    eprintln!("groovyfmt: {}: {e}", shown(path));
                    failed = true;
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
    eprintln!("{summary}");
    if failed {
        2
    } else {
        i32::from(check && changed > 0)
    }
}

fn lint_files(files: &[PathBuf], cfg: &Config, a: &Args, stats_only: bool) -> i32 {
    let t = Instant::now();
    let results = par_map(files, |p| {
        read(p).map(|src| src.and_then(|s| lint::lint(&s)))
    });
    let (mut skipped, mut total, mut failed) = (0usize, 0usize, false);
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for (path, r) in files.iter().zip(results) {
        match r {
            Err(e) => {
                eprintln!("groovyfmt: {e}");
                failed = true;
            }
            Ok(Err(why)) => {
                skipped += 1;
                if a.verbose {
                    eprintln!("skipped {}: {why}", shown(path));
                }
            }
            Ok(Ok(vs)) => {
                for v in vs.into_iter().filter(|v| cfg.rule_enabled(v.rule)) {
                    total += 1;
                    *by_rule.entry(v.rule).or_default() += 1;
                    if stats_only {
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
                        format!(
                            "{}:{}:{}: {}: {message}",
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
    let mut summary = format!("{total} violations in {} files", files.len());
    if !by_rule.is_empty() {
        let parts: Vec<_> = by_rule.iter().map(|(r, n)| format!("{r} {n}")).collect();
        summary += &format!(" ({})", parts.join(", "));
    }
    if skipped > 0 {
        summary += &format!(", {skipped} skipped (-v shows why)");
    }
    if a.verbose || stats_only {
        summary += &format!(" in {:.2}s", t.elapsed().as_secs_f64());
    }
    eprintln!("{summary}");
    if failed { 2 } else { i32::from(total > 0) }
}

/// Development: corpus report with an idempotence probe.
fn stats(files: &[PathBuf], cfg: &Config) -> i32 {
    let t = Instant::now();
    let results = par_map(files, |p| {
        let src = match read(p)? {
            Ok(src) => src,
            Err(why) => return Ok((0, Outcome::Skipped(why), false, 0)),
        };
        let out = format(&src, &cfg.style);
        let nonidem =
            matches!(&out, Outcome::Formatted(o) if format(o, &cfg.style) != Outcome::Unchanged);
        let lines = match &out {
            Outcome::Formatted(o) => {
                src.lines().zip(o.lines()).filter(|(a, b)| a != b).count()
                    + src.lines().count().abs_diff(o.lines().count())
            }
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
        let seq = TMP_SEQ.load(Ordering::Relaxed);
        let squat = d.join(format!(".A.groovy.groovyfmt-{}-{seq}", std::process::id()));
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
    fn gh_escape_properties_and_messages() {
        assert_eq!(gh_escape("a:b,c%\r\n", true), "a%3Ab%2Cc%25%0D%0A");
        assert_eq!(gh_escape("a:b,c%\n", false), "a:b,c%25%0A");
    }
}
