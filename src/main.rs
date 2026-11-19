use groovyfmt::baseline::{self, Baseline};
use groovyfmt::config::{self, Config, Resolver};
use groovyfmt::fileio::{FORCE, read, read_as, write_atomic, write_new};
use groovyfmt::fix::{self, Fix};
use groovyfmt::format::{Outcome, Style, format, format_lines};
use groovyfmt::git::git_changed;
use groovyfmt::lint::{self, RULES};
use groovyfmt::output::{
    Diag, Out, OutputFormat, PARSE_ERROR, diag_if, die, lint_diagnostic, parse_diagnostics,
    patch_for, print_raw, report_syntax, say, shown, to_stderr,
};
use groovyfmt::printable;
use groovyfmt::report;
use groovyfmt::syntax;
use groovyfmt::walk::{Configs, WALK_FAILED, WALK_VERBOSE, collect, par_map};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

const HELP: &str = "\
groovyfmt: formatter and linter for Groovy, Gradle build scripts, and Jenkinsfiles

Usage: groovyfmt [OPTIONS] [PATH]...

PATHs are files or directories (default: the current directory). Directories are
searched for *.groovy, *.gradle, *.gvy, *.gy, *.gsh, Jenkinsfile, *.jenkinsfile,
Jenkinsfile.*, Jenkinsfile_*, Jenkinsfile-*, *-Jenkinsfile, *_Jenkinsfile and
*.jenkins (those if they parse), and extensionless scripts whose first line is
a groovy #! line, skipping hidden directories (but not .ci, .jenkins, .github,
.gitlab, .circleci, .buildkite, .pipelines, .azure-pipelines), out, target,
node_modules, build (except a package named build under a src directory within
its project), and anything matching the config file's `exclude` patterns
(which also apply to files named here).
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
      --import-codenarc <FILE>...
                           Print a .groovyfmt.toml [lint] table that carries over a
                           CodeNarc configuration (ruleset XML or Groovy, codenarc.properties,
                           .groovylintrc.json), and what it can't carry over; with -v, each
                           rule's setting and where it comes from (stderr)

Options:
      --config <FILE>      Use FILE for every file instead of the nearest config
      --indent <N>         Spaces per indent level, or the tab width with use_tabs
                           (overrides the config file)
      --output-format <F>  text (default), github (GitHub Actions annotations), json
                           (--check, --lint, --syntax, in place: one JSON document on
                           stdout, schema in docs/output-formats.md) or sarif (--lint,
                           --syntax)
      --rules              List lint rules and whether each is on, then exit
      --explain <RULE>     Explain a lint rule: why it matters, an example, whether
                           --fix fixes it and how to turn it off
      --strict             List skipped files (unparseable, not UTF-8, too large) and
                           exit 1 if there are any, or if no Groovy file was found, so
                           CI can't pass on unchecked files
      --force              Read every file named here as Groovy, whatever its name
                           (by default one that isn't *.groovy, *.gradle, *.gvy, *.gy,
                           *.gsh, Jenkinsfile, Jenkinsfile.*, Jenkinsfile_*,
                           Jenkinsfile-*, *-Jenkinsfile, *_Jenkinsfile,
                           *.jenkinsfile, *.jenkins or a groovy #! script is
                           skipped)
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
      --fix                With --lint: fix in place what docs/lint.md marks fixable
                           (imports, semicolons, removed Gradle syntax and more; violations
                           marked \"(fixable)\"), then report what remains
  -v, --verbose            Also report skipped files and directories, where each file
                           failed to parse, and timing
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
    /// `--output-format`
    format: OutputFormat,
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
    ImportCodenarc,
    // development diagnostics, not in --help
    Stats,
    LintStats,
    Errors,
    Tree,
}

/// `--flag=value` whose value isn't UTF-8 (a path): the flag, and the value as it is.
#[cfg(unix)]
fn flag_and_value(arg: &std::ffi::OsStr) -> Option<(String, OsString)> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let b = arg.as_bytes();
    let eq = b.iter().position(|&c| c == b'=')?;
    let flag = std::str::from_utf8(&b[..eq]).ok()?;
    flag.starts_with("--")
        .then(|| (flag.to_string(), OsString::from_vec(b[eq + 1..].to_vec())))
}

/// Windows arguments that aren't Unicode can't be split safely: an error, as before.
#[cfg(not(unix))]
fn flag_and_value(_: &std::ffi::OsStr) -> Option<(String, OsString)> {
    None
}

fn parse_args() -> Args {
    let mut a = Args::default();
    // paths needn't be UTF-8 (a Latin-1 name from `xargs`); options and their text are
    let mut it = std::env::args_os().skip(1);
    let set_mode = |a: &mut Args, m: Mode| {
        // --lint --stats is the one allowed combination
        a.mode = match (std::mem::take(&mut a.mode), m) {
            (Mode::Write, m) => m,
            (Mode::Lint, Mode::Stats) | (Mode::Stats, Mode::Lint) => Mode::LintStats,
            _ => die(
                "--check, --lint, --syntax, --stdin, --rules, --suggest-style and --import-codenarc can't be combined",
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
        let (arg, flag, mut inline) = match arg.into_string() {
            Ok(arg) if arg == "--" => {
                only_paths = true;
                continue;
            }
            Ok(arg) => match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => {
                    (arg.clone(), f.to_string(), Some(OsString::from(v)))
                }
                _ => (arg.clone(), arg, None),
            },
            Err(arg) => match flag_and_value(&arg) {
                Some((f, v)) => (f.clone(), f, Some(v)),
                None if arg.to_string_lossy().starts_with('-') => die(format!(
                    "`{}` is not UTF-8 (see --help)",
                    printable(&arg.to_string_lossy())
                )),
                None => {
                    a.paths.push(PathBuf::from(arg));
                    continue;
                }
            },
        };
        // a path option's value may be any OsString; `text` is for the others
        let mut value = |name: &str| {
            inline
                .take()
                .or_else(|| it.next())
                .unwrap_or_else(|| die(format!("{name} needs a value")))
        };
        let text = |name: &str, v: OsString| {
            v.into_string()
                .unwrap_or_else(|_| die(format!("{name} needs a UTF-8 value")))
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
                let rule = text("--explain", value("--explain"));
                set_mode(&mut a, Mode::Explain(rule));
            }
            "--suggest-style" => set_mode(&mut a, Mode::SuggestStyle),
            "--import-codenarc" => set_mode(&mut a, Mode::ImportCodenarc),
            "--lsp" => set_mode(&mut a, Mode::Lsp),
            "--stats" => set_mode(&mut a, Mode::Stats),
            "--errors" => set_mode(&mut a, Mode::Errors),
            "--tree" => set_mode(&mut a, Mode::Tree),
            "-v" | "--verbose" => a.verbose = true,
            "--strict" => {
                a.strict = true;
                config::STRICT.store(true, Ordering::Relaxed);
            }
            "--fix" => a.fix = true,
            "--force" => FORCE.store(true, Ordering::Relaxed),
            "--diff" => a.diff = true,
            "--lines" => {
                let v = text("--lines", value("--lines"));
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
            "--changed-since" => {
                a.changed_since = Some(text("--changed-since", value("--changed-since")))
            }
            "--baseline" => a.baseline = Some(PathBuf::from(value("--baseline"))),
            // only `--write-baseline=FILE`: a separate word would read as a path to lint
            "--write-baseline" => a.write_baseline = Some(inline.take().map(PathBuf::from)),
            "--config" => a.config = Some(PathBuf::from(value("--config"))),
            "--stdin-filepath" => a.stdin_filepath = Some(PathBuf::from(value("--stdin-filepath"))),
            "--indent" => {
                let v = text("--indent", value("--indent"));
                a.indent = Some(
                    v.parse()
                        .ok()
                        .filter(|n| (1..=16).contains(n))
                        .unwrap_or_else(|| die("--indent must be 1 to 16")),
                );
            }
            "--output-format" => match text("--output-format", value("--output-format")).as_str() {
                "text" => a.format = OutputFormat::Text,
                "github" => a.format = OutputFormat::Github,
                "json" => a.format = OutputFormat::Json,
                "sarif" => a.format = OutputFormat::Sarif,
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
    match (a.format, &a.mode) {
        (OutputFormat::Json, Mode::Write | Mode::Check | Mode::Lint | Mode::Syntax)
        | (OutputFormat::Sarif, Mode::Lint | Mode::Syntax)
        | (OutputFormat::Text | OutputFormat::Github, _) => {}
        (OutputFormat::Json, _) => {
            die("--output-format json works with --check, --lint, --syntax and formatting in place")
        }
        (OutputFormat::Sarif, _) => die("--output-format sarif works with --lint and --syntax"),
    }
    if a.fix && a.mode != Mode::Lint {
        die("--fix needs --lint");
    }
    if (a.baseline.is_some() || a.write_baseline.is_some()) && a.mode != Mode::Lint {
        die("--baseline and --write-baseline need --lint");
    }
    if a.write_baseline.is_some() && (a.fix || a.format != OutputFormat::Text) {
        die("--write-baseline can't be combined with --fix or --output-format");
    }
    if a.diff && !(a.mode == Mode::Check || a.fix) {
        die("--diff works with --check and --lint --fix");
    }
    if a.diff && a.format != OutputFormat::Text {
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
    if a.mode == Mode::ImportCodenarc && a.paths.is_empty() {
        die("--import-codenarc needs the CodeNarc configuration files to read");
    }
    if a.paths.is_empty() && !matches!(a.mode, Mode::Stdin | Mode::Lsp) {
        a.paths.push(PathBuf::from("."));
    }
    a
}

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

/// `format`, or with `--lines` only the lines in those ranges.
fn format_some(src: &str, style: &Style, lines: &[(usize, usize)]) -> Outcome {
    if lines.is_empty() {
        format(src, style)
    } else {
        format_lines(src, style, lines)
    }
}

fn main() {
    // a panic message can quote source text (a bad string slice prints the string), and
    // par_map turns the panic into an error but the hook still prints it first
    std::panic::set_hook(Box::new(|info| {
        eprintln!("groovyfmt: {}", printable(&info.to_string()));
    }));
    let a = parse_args();
    if a.mode == Mode::ImportCodenarc {
        std::process::exit(import_codenarc(&a));
    }
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
            WALK_VERBOSE.store(
                a.verbose && !matches!(a.mode, Mode::Errors | Mode::Tree),
                Ordering::Relaxed,
            );
            let (mut files, links, cfgs) = collect(&a.paths, &mut res, write);
            if let Some(rev) = &a.changed_since {
                let changed = git_changed(rev);
                files.retain(|f| std::fs::canonicalize(f).is_ok_and(|c| changed.contains(&c)));
            }
            if a.verbose && !matches!(a.mode, Mode::Errors | Mode::Tree) {
                report(files.iter().map(|f| &*cfgs[f]));
            }
            // a walk that found nothing (every pipeline in a skipped directory, a typo'd
            // path) would pass CI having checked nothing; with --changed-since an empty
            // list is a clean change set (#202)
            let nothing = a.strict && files.is_empty() && a.changed_since.is_none();
            if nothing {
                eprintln!("groovyfmt: --strict: no Groovy files found");
            }
            let code = match a.mode {
                Mode::Lint => {
                    // where --write-baseline writes: its own value, else --baseline, else the
                    // working directory's config, else the default name
                    let writing = a.write_baseline.as_ref().map(|w| {
                        let given = w
                            .clone()
                            .or_else(|| a.baseline.clone().filter(|b| !b.as_os_str().is_empty()));
                        if let Some(f) = given {
                            return f;
                        }
                        // one the config names must stay in the config's directory
                        let cfg = one_config(&mut res, &a);
                        match &cfg.baseline {
                            Some(f) => {
                                baseline::confined(&cfg.root, f).unwrap_or_else(|e| die(e));
                                f.clone()
                            }
                            None => PathBuf::from(baseline::DEFAULT_FILE),
                        }
                    });
                    lint_files(
                        &files,
                        &links,
                        &cfgs,
                        &a,
                        false,
                        writing.as_deref(),
                        res.fixed(),
                    )
                }
                Mode::Syntax => syntax(&files, &a),
                Mode::LintStats => lint_files(&files, &links, &cfgs, &a, true, None, res.fixed()),
                Mode::Stats => stats(&files, &cfgs),
                Mode::SuggestStyle => suggest_style(&files, &cfgs),
                Mode::Errors => errors(&files),
                Mode::Tree => tree(&files),
                _ => fmt(&files, &links, &cfgs, &a),
            };
            if nothing { code.max(1) } else { code }
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
    } else if lint::PARTLY_FIXABLE.contains(&r.name) {
        "Partly fixed automatically (see lint.md#fixing)."
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

/// `--import-codenarc`: the config on stdout, with `-v` each rule's source on stderr.
fn import_codenarc(a: &Args) -> i32 {
    let im = groovyfmt::codenarc::import(&a.paths).unwrap_or_else(|e| die(e));
    // text from the files (values, statements in notes) can't carry terminal controls
    let shown = |s: String| s.lines().map(|l| printable(l) + "\n").collect::<String>();
    if a.verbose {
        eprint!("{}", shown(im.details()));
    }
    print_raw(&shown(im.toml()));
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
    let formatted = format_some(&src, &cfg.style, &a.lines);
    if let Some(why) = groovyfmt::wrap::take_fallback().filter(|_| a.verbose) {
        let name = path.map_or_else(|| "<stdin>".to_string(), shown);
        eprintln!("not wrapped {name}: {why}");
    }
    let out = match formatted {
        Outcome::Unchanged => src,
        Outcome::Formatted(out) => out,
        Outcome::Skipped(why) => {
            let name = path.map_or_else(|| "<stdin>".to_string(), shown);
            let github = a.format == OutputFormat::Github;
            if github {
                let message = format!("Not formatted: {why}");
                eprintln!("{}", report::github("error", &name, None, None, &message));
            } else {
                eprintln!("groovyfmt: {name}: not formatted: {why}");
            }
            report_syntax(&name, &syntax::check(&src), github, to_stderr);
            std::process::exit(2);
        }
    };
    std::io::stdout().write_all(out.as_bytes()).map_or(2, |_| 0)
}

fn fmt(files: &[PathBuf], links: &HashMap<PathBuf, PathBuf>, cfgs: &Configs, a: &Args) -> i32 {
    let t = Instant::now();
    let check = a.mode == Mode::Check;
    let mut out = Out::new(a.format);
    let listed = (a.verbose || a.strict) && out.text();
    let want_diag = listed || !out.text();
    // Each worker writes (or, for --check, drops) the new text itself, so no more than one
    // output per thread is alive at once; only the small per-file results are kept, and
    // they're reported below in input order.
    let results = par_map(files, |p| {
        let real = links.get(p).map_or(p, PathBuf::as_path);
        let (src, id) = match read_as(real, p)? {
            Ok(read) => read,
            Err(why) => return Ok((Err((why, Diag::default())), None)),
        };
        let formatted = format_some(&src, &cfgs[p].style, &a.lines);
        let fallback = groovyfmt::wrap::take_fallback();
        let r = match formatted {
            Outcome::Unchanged => Ok(None),
            Outcome::Skipped(why) => Err((why, diag_if(want_diag, &src))),
            Outcome::Formatted(out) if check => {
                Ok(Some((Ok(()), a.diff.then(|| patch_for(p, &src, &out)))))
            }
            Outcome::Formatted(out) => Ok(Some((write_atomic(real, &out, id), None))),
        };
        Ok((r, fallback))
    });
    let (mut changed, mut skipped, mut failed) = (0usize, 0usize, false);
    for (path, r) in files.iter().zip(results) {
        // `max_line_length` was set but the file was formatted without wrapping: worth knowing
        let r = r.map(|(r, fallback)| {
            if let Some(why) = fallback.filter(|_| listed) {
                eprintln!("not wrapped {}: {why}", shown(path));
            }
            r
        });
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
                // a file whose write failed is an error, not reformatted
                if let Err(e) = written {
                    eprintln!("groovyfmt: {}: {e}", shown(path));
                    failed = true;
                    out.error(path, &e.to_string());
                    continue;
                }
                changed += 1;
                if let Some(patch) = patch {
                    print_raw(&patch);
                } else if !out.text() {
                    let status = if check {
                        "would_reformat"
                    } else {
                        "reformatted"
                    };
                    out.file(path, status, None, Vec::new());
                } else if check && a.format == OutputFormat::Github {
                    let message = "File is not formatted. Run groovyfmt to fix.";
                    say(&report::github("error", &shown(path), None, None, message));
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
    keys: Option<(String, baseline::Keys)>,
}

/// The baseline file for a file governed by `cfg`: `--baseline` (`''` for none), else the
/// config's, which must stay in the config's directory (an error, exit 2, otherwise).
fn baseline_file(a: &Args, cfg: &Config) -> Option<PathBuf> {
    baseline::file_for(cfg, a.baseline.as_deref()).unwrap_or_else(|e| die(e))
}

fn lint_files(
    files: &[PathBuf],
    links: &HashMap<PathBuf, PathBuf>,
    cfgs: &Configs,
    a: &Args,
    stats_only: bool,
    writing: Option<&Path>,
    fixed: Option<Config>,
) -> i32 {
    let t = Instant::now();
    // each project's other files, which may use a private member reported as unused
    let projects = lint::Projects::new(fixed);
    let mut out = Out::new(a.format);
    // each file's baseline and its path in it; each baseline file is read once, and one that
    // can't be read or isn't a valid baseline is an error
    let mut loaded: HashMap<PathBuf, Arc<Baseline>> = HashMap::new();
    let mut load = |f: &Path| {
        loaded
            .entry(f.to_path_buf())
            .or_insert_with(|| Arc::new(Baseline::load(f).unwrap_or_else(|e| die(e))))
            .clone()
    };
    let bases: HashMap<&Path, (Arc<Baseline>, String)> = match writing {
        // writing: the whole of every file is recorded, relative to the file written
        Some(target) => files
            .iter()
            .filter_map(|p| {
                let Some(rel) = baseline::name_in(p, target) else {
                    eprintln!(
                        "groovyfmt: warning: {}: the name isn't UTF-8, so its violations aren't recorded in the baseline",
                        shown(p)
                    );
                    return None;
                };
                Some((p.as_path(), (Arc::new(Baseline::default()), rel)))
            })
            .collect(),
        // a file whose name isn't UTF-8 has no entries: all of its violations are reported
        None => files
            .iter()
            .filter_map(|p| {
                let f = baseline_file(a, &cfgs[p])?;
                Some((p.as_path(), (load(&f), baseline::name_in(p, &f)?)))
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
        let cfg = &cfgs[p];
        let violations = match lint::run(p, real, &src, cfg, &projects) {
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
            // a violation the baseline has on a line like this one is left alone
            let base = bases.get(&p);
            let wanted = |v: &lint::Violation, text: &str| {
                cfg.rule_enabled(v.rule)
                    && !base.is_some_and(|(b, rel)| b.covers_line(rel, v.rule, text, v.line))
            };
            let compile = cfg.gradle_compile_replacement.as_deref();
            match fix::fix(p, &src, compile, &projects.gradle_context(), wanted) {
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
                        r.violations =
                            lint::run(p, real, &text, cfg, &projects).unwrap_or_default();
                        r.fixed = fixed;
                        new_text = Some(text);
                    }
                }
            }
        }
        let text = new_text.as_deref().unwrap_or(&src);
        if let Some((b, rel)) = bases.get(&p) {
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
                    entries.extend(
                        keys.into_iter()
                            .map(|(rule, fp, i)| (rel.clone(), rule, fp, i)),
                    );
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
                let vs = r.violations;
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
                    say(&if a.format == OutputFormat::Github {
                        let at = Some((v.line, v.col));
                        report::github("warning", &shown(path), at, Some(v.rule), &message)
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
    if !bases.is_empty() {
        out.extra = vec![("baselined", held), ("stale_baseline_entries", stale)];
    }
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

/// `--syntax`: parse only, and report every file that doesn't parse cleanly.
fn syntax(files: &[PathBuf], a: &Args) -> i32 {
    let t = Instant::now();
    let results = par_map(files, |p| read(p).map(|src| src.map(|s| syntax::check(&s))));
    let mut out = Out::new(a.format);
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
                    report_syntax(&shown(path), &diag, a.format == OutputFormat::Github, say);
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
            // indent styles only: wrapping would make every candidate change long lines
            let style = Style {
                indent,
                continuation,
                use_tabs,
                max_line_length: None,
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
