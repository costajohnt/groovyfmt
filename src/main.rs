use groovyfmt::format::{Outcome, format};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

const USAGE: &str = "usage: groovyfmt [--check | --stats | --lint [--stats]] [-v] <files or dirs...>";

fn walk(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let Ok(rd) = std::fs::read_dir(p) else { return };
        for e in rd.flatten() {
            let path = e.path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            walk(&path, out);
        }
    } else if groovyfmt::is_groovy_file(p) {
        out.push(p.to_path_buf());
    }
}

/// Run `f` over every file on all cores; results come back in input order.
fn par_map<R: Send>(files: &[PathBuf], f: impl Fn(&Path) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut out: Vec<(usize, R)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = files.get(i) else { break };
                        mine.push((i, f(path)));
                    }
                    mine
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker thread panicked")).collect()
    });
    out.sort_unstable_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let (stats, check, verbose) = (flag("--stats"), flag("--check"), flag("-v"));
    let mut files = Vec::new();
    for a in args.iter().filter(|a| !a.starts_with('-')) {
        walk(Path::new(a), &mut files);
    }
    if files.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let t = Instant::now();
    let code = if flag("--tree") {
        tree(&files)
    } else if flag("--errors") {
        errors(&files)
    } else if flag("--lint") {
        lint(&files, stats, verbose)
    } else {
        fmt(&files, stats, check, verbose)
    };
    if verbose {
        eprintln!("{:.2}s", t.elapsed().as_secs_f64());
    }
    std::process::exit(code);
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn fmt(files: &[PathBuf], stats: bool, check: bool, verbose: bool) -> i32 {
    let t = Instant::now();
    let results = par_map(files, |p| {
        read(p).map(|src| {
            let out = format(&src);
            // idempotence probe only in --stats, so it stays off the normal path
            let idem = stats && matches!(&out, Outcome::Formatted(o) if format(o) != Outcome::Unchanged);
            let changed_lines = match &out {
                Outcome::Formatted(o) if stats => {
                    src.lines().zip(o.lines()).filter(|(a, b)| a != b).count() + src.lines().count().abs_diff(o.lines().count())
                }
                _ => 0,
            };
            (src.len(), out, idem, changed_lines)
        })
    });
    let (mut bytes, mut changed, mut unchanged, mut nonidem, mut lines_changed, mut failed) = (0, 0, 0, 0, 0, false);
    let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
    for (path, r) in files.iter().zip(results) {
        let (len, out, idem, cl) = match r {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{e}");
                failed = true;
                continue;
            }
        };
        bytes += len;
        lines_changed += cl;
        match out {
            Outcome::Unchanged => unchanged += 1,
            Outcome::Skipped(why) => {
                *skipped.entry(why).or_default() += 1;
                if verbose || !stats {
                    eprintln!("skipped {}: {why}", path.display());
                }
            }
            Outcome::Formatted(out) => {
                changed += 1;
                if idem {
                    nonidem += 1;
                    if verbose {
                        println!("NONIDEM {}", path.display());
                    }
                }
                if check {
                    println!("{}", path.display());
                } else if !stats && let Err(e) = std::fs::write(path, out) {
                    eprintln!("{}: {e}", path.display());
                    failed = true;
                }
            }
        }
    }
    let secs = t.elapsed().as_secs_f64();
    if stats {
        println!(
            "files={} unchanged={unchanged} formatted={changed} lines_changed={lines_changed} nonidempotent={nonidem} skipped={skipped:?} MB={:.1} secs={secs:.2} MB/s={:.1}",
            files.len(),
            bytes as f64 / 1e6,
            bytes as f64 / 1e6 / secs.max(1e-9)
        );
    } else if !check {
        eprintln!("formatted {changed} of {} files in {secs:.2}s", files.len());
    }
    i32::from(failed || (check && changed > 0))
}

fn lint(files: &[PathBuf], stats: bool, verbose: bool) -> i32 {
    let t = Instant::now();
    let results = par_map(files, |p| read(p).map(|src| groovyfmt::lint::lint(&src)));
    let (mut skipped, mut total, mut failed) = (0usize, 0usize, false);
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for (path, r) in files.iter().zip(results) {
        match r {
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
            Ok(Err(why)) => {
                skipped += 1;
                if verbose {
                    eprintln!("skipped {}: {why}", path.display());
                }
            }
            Ok(Ok(vs)) => {
                for v in vs {
                    total += 1;
                    *by_rule.entry(v.rule).or_default() += 1;
                    if !stats {
                        println!("{}:{}:{}: {}: {}", path.display(), v.line, v.col, v.rule, v.message);
                    }
                }
            }
        }
    }
    eprintln!(
        "linted {} files ({skipped} skipped: parse error) in {:.2}s, {total} violations {by_rule:?}",
        files.len() - skipped,
        t.elapsed().as_secs_f64()
    );
    i32::from(failed || total > 0)
}

/// Diagnostics: "<parent kind>\t<ERROR|MISSING x>\t<line>\t<path:line>" per file that fails to parse.
fn errors(files: &[PathBuf]) -> i32 {
    let rows = par_map(files, |p| {
        let src = read(p).ok()?;
        let tree = groovyfmt::parse(&src);
        let e = groovyfmt::first_error(&tree)?;
        let line = src.lines().nth(e.start_position().row).unwrap_or("").trim();
        let parent = e.parent().map_or("-", |p| p.kind());
        let kind = if e.is_missing() { format!("MISSING {}", e.kind()) } else { "ERROR".into() };
        Some(format!("{parent}\t{kind}\t{}\t{}:{}", line.chars().take(100).collect::<String>(), p.display(), e.start_position().row + 1))
    });
    for r in rows.into_iter().flatten() {
        println!("{r}");
    }
    0
}

/// Debug: dump each file's parse tree.
fn tree(files: &[PathBuf]) -> i32 {
    for f in files {
        match read(f) {
            Ok(src) => println!("{}", groovyfmt::parse(&src).root_node().to_sexp()),
            Err(e) => eprintln!("{e}"),
        }
    }
    0
}
