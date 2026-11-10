use groovyfmt::config::{self, Config};
use groovyfmt::format::{Outcome, format};
use groovyfmt::lint::{self, RULES};
use std::collections::BTreeMap;
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

fn collect(paths: &[PathBuf], cfg: &Config) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for p in paths {
        if !p.exists() {
            die(format!("{}: no such file or directory", p.display()));
        }
        walk(p, true, &mut files, cfg);
    }
    files
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
    let err = |e: std::io::Error| format!("{}: {e}", shown(path));
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
    Ok(String::from_utf8(bytes).map_err(|_| NOT_UTF8))
}

/// Replace a file's contents atomically: write a temporary file beside it with the same
/// permissions, then rename it over the path. A crash can't leave a truncated file, and if
/// the path was swapped for a symlink after the walk, the link is replaced rather than
/// written through.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let perms = std::fs::metadata(path)?.permissions();
    // a rename would replace a read-only file too; read-only means hands off
    if perms.readonly() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "file is read-only",
        ));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.groovyfmt-{}", std::process::id()));
    let result = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        // owner-only until the original permissions are applied, so the contents of a
        // private file are never readable by others in between
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp)?;
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
        Mode::Write | Mode::Check => fmt(&collect(&a.paths, &cfg), &cfg, &a),
        Mode::Lint => lint_files(&collect(&a.paths, &cfg), &cfg, &a, false),
        Mode::LintStats => lint_files(&collect(&a.paths, &cfg), &cfg, &a, true),
        Mode::Stats => stats(&collect(&a.paths, &cfg), &cfg),
        Mode::Errors => errors(&collect(&a.paths, &cfg)),
        Mode::Tree => tree(&collect(&a.paths, &cfg)),
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

fn fmt(files: &[PathBuf], cfg: &Config, a: &Args) -> i32 {
    let t = Instant::now();
    let check = a.mode == Mode::Check;
    let results = par_map(files, |p| {
        read(p).map(|src| src.map_or_else(Outcome::Skipped, |s| format(&s, &cfg.style)))
    });
    let (mut changed, mut skipped, mut failed) = (0usize, 0usize, false);
    for (path, r) in files.iter().zip(results) {
        match r {
            Err(e) => {
                eprintln!("groovyfmt: {e}");
                failed = true;
            }
            Ok(Outcome::Unchanged) => {}
            Ok(Outcome::Skipped(why)) => {
                skipped += 1;
                if a.verbose {
                    eprintln!("skipped {}: {why}", shown(path));
                }
            }
            Ok(Outcome::Formatted(out)) => {
                changed += 1;
                if !check {
                    if let Err(e) = write_atomic(path, &out) {
                        eprintln!("groovyfmt: {}: {e}", shown(path));
                        failed = true;
                    }
                } else if a.github {
                    say(&format!(
                        "::error file={}::File is not formatted. Run groovyfmt to fix.",
                        gh_escape(&shown(path), true)
                    ));
                } else {
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

    #[test]
    fn printable_neutralizes_line_breaks() {
        assert_eq!(
            printable("a\n::set-env name=X::y\r"),
            "a\\n::set-env name=X::y\\r"
        );
        assert_eq!(printable("plain é"), "plain é");
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
    fn gh_escape_properties_and_messages() {
        assert_eq!(gh_escape("a:b,c%\r\n", true), "a%3Ab%2Cc%25%0D%0A");
        assert_eq!(gh_escape("a:b,c%\n", false), "a:b,c%25%0A");
    }
}
