use groovyfmt::format::{Outcome, format};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let (stats, check, verbose) = (flag("--stats"), flag("--check"), flag("-v"));
    if flag("--tree") {
        // debug: dump the parse tree of each file argument
        for a in args.iter().filter(|a| !a.starts_with('-')) {
            let src = std::fs::read_to_string(a).expect("read");
            println!("{}", groovyfmt::parse(&src).root_node().to_sexp());
        }
        return;
    }
    let mut files = Vec::new();
    for a in args.iter().filter(|a| !a.starts_with('-')) {
        walk(Path::new(a), &mut files);
    }
    if files.is_empty() {
        eprintln!("usage: groovyfmt [--check | --stats] [-v] <files or dirs...>");
        std::process::exit(2);
    }

    let t = Instant::now();
    let mut lines_changed = 0usize;
    let (mut bytes, mut changed, mut unchanged, mut nonidem, mut failed) = (0usize, 0usize, 0usize, 0usize, false);
    let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", f.display());
                failed = true;
                continue;
            }
        };
        bytes += src.len();
        match format(&src) {
            Outcome::Unchanged => unchanged += 1,
            Outcome::Skipped(why) => {
                *skipped.entry(why).or_default() += 1;
                if verbose || !stats {
                    eprintln!("skipped {}: {why}", f.display());
                }
            }
            Outcome::Formatted(out) => {
                changed += 1;
                if stats {
                    lines_changed += src.lines().zip(out.lines()).filter(|(a, b)| a != b).count()
                        + src.lines().count().abs_diff(out.lines().count());
                    if format(&out) != Outcome::Unchanged {
                        nonidem += 1;
                        if verbose {
                            println!("NONIDEM {}", f.display());
                        }
                    }
                } else if check {
                    println!("{}", f.display());
                } else if let Err(e) = std::fs::write(f, out) {
                    eprintln!("{}: {e}", f.display());
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
    if failed || (check && changed > 0) {
        std::process::exit(1);
    }
}
