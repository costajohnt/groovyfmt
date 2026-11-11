//! The corpus invariants, over the files in tests/fixtures: formatting changes whitespace
//! only, never skips these files, and a second pass changes nothing. Checked on each fixture
//! as written and on a copy with every line's indent scrambled, at three styles.

use groovyfmt::format::{Outcome, Style, format};

fn no_space(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Every line's leading whitespace replaced by a varying run of spaces and tabs.
fn scramble(src: &str) -> String {
    src.split('\n')
        .enumerate()
        .map(|(i, l)| {
            let body = l.trim_start_matches([' ', '\t']);
            if body.is_empty() || body == "\r" {
                return l.to_string();
            }
            format!("{}{body}", [" ", "\t", "   ", "", "  \t "][i % 5])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The formatted text; panics unless the invariants hold.
fn check(name: &str, src: &str, style: &Style) -> String {
    let out = match format(src, style) {
        Outcome::Unchanged => src.to_string(),
        Outcome::Formatted(o) => o,
        Outcome::Skipped(why) => panic!("{name}: skipped: {why}"),
    };
    assert_eq!(no_space(&out), no_space(src), "{name}: not whitespace-only");
    assert_eq!(
        format(&out, style),
        Outcome::Unchanged,
        "{name}: not idempotent"
    );
    out
}

/// Small edits to formatted text, of the kind a person makes between two runs: one space
/// deleted, one added before a `:` or `=`, a trailing `;` removed, one line deleted; each at
/// up to `n` places spread over the file.
fn mutants(text: &str, n: usize) -> Vec<String> {
    let spread = |at: Vec<usize>| -> Vec<usize> {
        let m = n.min(at.len());
        let mut v: Vec<usize> = (0..m)
            .map(|i| at[(2 * i + 1) * at.len() / (2 * m)])
            .collect();
        v.dedup();
        v
    };
    let edit =
        |at: usize, del: usize, ins: &str| format!("{}{ins}{}", &text[..at], &text[at + del..]);
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let spaces: Vec<usize> = (0..bytes.len()).filter(|&i| bytes[i] == b' ').collect();
    out.extend(spread(spaces).into_iter().map(|i| edit(i, 1, "")));
    let colons: Vec<usize> = (0..bytes.len())
        .filter(|&i| matches!(bytes[i], b':' | b'='))
        .collect();
    out.extend(spread(colons).into_iter().map(|i| edit(i, 0, " ")));
    let mut line_starts = vec![0];
    line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    line_starts.pop_if(|&mut s| s == text.len());
    let semis: Vec<usize> = line_starts
        .iter()
        .filter_map(|&s| {
            let line = &text[s..text[s..].find('\n').map_or(text.len(), |e| s + e)];
            let t = line.trim_end();
            t.ends_with(';').then(|| s + t.len() - 1)
        })
        .collect();
    out.extend(spread(semis).into_iter().map(|i| edit(i, 1, "")));
    for s in spread(line_starts) {
        let e = text[s..].find('\n').map_or(text.len(), |e| s + e + 1);
        out.push(edit(s, e - s, ""));
    }
    out
}

/// Formatted text, or None when the file is skipped.
fn formatted(src: &str, style: &Style) -> Option<String> {
    match format(src, style) {
        Outcome::Unchanged => Some(src.to_string()),
        Outcome::Formatted(o) => Some(o),
        Outcome::Skipped(_) => None,
    }
}

/// Whether formatting `src` gives a fixed point: formatting the result changes nothing.
fn settles(src: &str, style: &Style) -> bool {
    formatted(src, style).is_none_or(|out| format(&out, style) == Outcome::Unchanged)
}

/// The edits in `mutants` made to `name`'s formatted text that don't settle in one pass.
fn unsettled_mutants(src: &str, style: &Style, n: usize) -> Vec<String> {
    let Some(out) = formatted(src, style) else {
        return Vec::new();
    };
    mutants(&out, n)
        .into_iter()
        .filter(|m| !settles(m, style))
        .collect()
}

/// The styles the mutation checks run at.
fn mutation_styles() -> [(&'static str, Style); 3] {
    let spacing = Style {
        normalize_spacing: true,
        ..Style::default()
    };
    let wrap = Style {
        max_line_length: Some(120),
        ..Style::default()
    };
    let both = Style {
        max_line_length: Some(120),
        ..spacing
    };
    [("spacing", spacing), ("wrap", wrap), ("both", both)]
}

/// The fixture directory: GROOVYFMT_TEST_FIXTURES when the test binary runs on another machine.
fn fixture_paths() -> (String, Vec<std::path::PathBuf>) {
    let dir = std::env::var("GROOVYFMT_TEST_FIXTURES")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures").into());
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    (dir, paths)
}

#[test]
fn edited_output_settles_in_one_pass() {
    // A second run must change nothing, also after someone edits the output a little: a
    // decision that depends on how a line was written, rather than on the text it becomes,
    // passes the plain idempotence check on a fresh file and fails here.
    let (_, paths) = fixture_paths();
    let mut tried = 0;
    for p in &paths {
        let name = p.file_name().unwrap().to_string_lossy();
        let src = std::fs::read_to_string(p).unwrap();
        for (style_name, style) in mutation_styles() {
            let bad = unsettled_mutants(&src, &style, 6);
            assert!(bad.is_empty(), "{name} at {style_name}:\n{}", bad[0]);
            tried += 1;
        }
    }
    assert!(tried >= 60);
    // the case that found it, reduced (from streamaserver/streama's application.groovy): a
    // row held still against a row that then changed kept its old spacing
    let rows = "x = [\n  [pattern:'/userActivity/**',  access :['A']],\n  [pattern:'/monitoring/**',  access :['A']],\n  [pattern:'/**/favicon.ico', access :['B']],\n  [pattern:'/api/currentUser',  access :['B']],\n  [pattern:'/api/v1/dash/**',  access :['C']]\n]\n";
    let style = &mutation_styles()[0].1;
    assert!(settles(rows, style));
    assert!(unsettled_mutants(rows, style, 20).is_empty());
}

/// The mutation check over every Groovy file under the directories in
/// GROOVYFMT_MUTATION_DIRS (separated by `:`), at each style in `mutation_styles`, and
/// `--lint --fix` followed by formatting at every combination of `normalize_spacing` and
/// `max_line_length`. Prints the counts, and writes the failing files to
/// GROOVYFMT_MUTATION_OUT if set. Run with `cargo test --release -- --ignored mutation`.
#[test]
#[ignore = "runs over a corpus named in GROOVYFMT_MUTATION_DIRS"]
fn mutation_check_over_a_corpus() {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    let dirs = std::env::var("GROOVYFMT_MUTATION_DIRS").expect("GROOVYFMT_MUTATION_DIRS");
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = dirs.split(':').map(PathBuf::from).collect();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && !groovyfmt::is_skipped_dir(&p) {
                stack.push(p);
            } else if t.is_file() && groovyfmt::is_groovy_file(&p) {
                files.push(p);
            }
        }
    }
    files.sort();
    let pipeline: Vec<(String, Style)> = [false, true]
        .into_iter()
        .flat_map(|s| {
            [None, Some(120)].map(|m| {
                let st = Style {
                    normalize_spacing: s,
                    max_line_length: m,
                    ..Style::default()
                };
                (format!("fix+fmt spacing={s} wrap={m:?}"), st)
            })
        })
        .collect();
    let checks: Vec<(String, Style)> = mutation_styles()
        .into_iter()
        .map(|(n, s)| (n.to_string(), s))
        .collect();
    let all = checks.len() + pipeline.len();
    let failed: Mutex<Vec<Vec<String>>> = Mutex::new(vec![Vec::new(); all]);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let one = |p: &Path| {
        let meta = std::fs::metadata(p).ok()?;
        if meta.len() > groovyfmt::MAX_FILE_BYTES {
            return None;
        }
        let src = String::from_utf8(std::fs::read(p).ok()?).ok()?;
        let mut bad = vec![false; all];
        for (i, (_, style)) in checks.iter().enumerate() {
            bad[i] = !settles(&src, style) || !unsettled_mutants(&src, style, 2).is_empty();
        }
        let fixed = match groovyfmt::fix::fix(p, &src, |_, _| true) {
            groovyfmt::fix::Fix::Fixed(t, _) => t,
            _ => src,
        };
        for (i, (_, style)) in pipeline.iter().enumerate() {
            bad[checks.len() + i] = !settles(&fixed, style);
        }
        Some(bad)
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(p) = files.get(i) else { break };
                    let Some(bad) = one(p) else { continue };
                    let mut f = failed.lock().unwrap();
                    for (k, b) in bad.into_iter().enumerate() {
                        if b {
                            f[k].push(p.display().to_string());
                        }
                    }
                }
            });
        }
    });
    let failed = failed.into_inner().unwrap();
    let mut report = String::new();
    for ((name, _), f) in checks.iter().chain(&pipeline).zip(&failed) {
        eprintln!("{name}: {} of {} files fail", f.len(), files.len());
        for p in f {
            report.push_str(&format!("{name}\t{p}\n"));
        }
    }
    if let Ok(out) = std::env::var("GROOVYFMT_MUTATION_OUT") {
        std::fs::write(out, report).unwrap();
    }
    assert!(failed.iter().all(Vec::is_empty));
}

#[test]
fn fixtures_format_whitespace_only_and_idempotently() {
    let (dir, paths) = fixture_paths();
    let dir = dir.as_str();
    let styles = [
        Style::default(),
        Style {
            indent: 2,
            continuation: Some(8),
            indent_after_labels: true,
            ..Style::default()
        },
        Style {
            use_tabs: true,
            ..Style::default()
        },
        Style {
            normalize_spacing: true,
            ..Style::default()
        },
        // wrapping only adds line breaks: still whitespace-only and idempotent
        Style {
            max_line_length: Some(80),
            ..Style::default()
        },
        Style {
            max_line_length: Some(60),
            normalize_spacing: true,
            use_tabs: true,
            continuation: Some(8),
            ..Style::default()
        },
    ];
    let mut changed_when_scrambled = 0;
    for p in &paths {
        let name = p.file_name().unwrap().to_string_lossy();
        let src = std::fs::read_to_string(p).unwrap();
        let scrambled = scramble(&src);
        for style in &styles {
            check(&name, &src, style);
            if check(&format!("{name} (scrambled)"), &scrambled, style) != scrambled
                && *style == Style::default()
            {
                changed_when_scrambled += 1;
                eprintln!("changes when scrambled: {name}");
            }
        }
    }
    assert!(paths.len() >= 20, "{} fixtures", paths.len());
    // spacing changes only what the option is for, and never moves a line break
    let src = std::fs::read_to_string(format!("{dir}/spacing.groovy")).unwrap();
    let plain = check("spacing.groovy", &src, &Style::default());
    let spaced = check("spacing.groovy", &src, &styles[3]);
    assert_ne!(plain, spaced, "spacing.groovy: spacing changed nothing");
    assert_eq!(plain.lines().count(), spaced.lines().count());
    for (a, b) in plain.lines().zip(spaced.lines()) {
        assert_eq!(a.replace([' ', '\t'], ""), b.replace([' ', '\t'], ""));
    }
    for line in [
        "        a  | b  || c",
        "        a -b",
        "        foo (x).bar()",
        "        y [1]",
        "        println \"keep ${a+b}   as is\"",
        "   c  =  d'''  +'e'",
        "    String  name      // aligned",
        "        if (a) { println a } else if (b) { println b } else { println 'none' }",
    ] {
        assert!(
            spaced.contains(&format!("{line}\n")),
            "{line:?} in\n{spaced}"
        );
    }
    // wrap.groovy: long lines are wrapped where that is safe, everything else is left long
    let src = std::fs::read_to_string(format!("{dir}/wrap.groovy")).unwrap();
    let plain = check("wrap.groovy", &src, &Style::default());
    let wrapped = check("wrap.groovy", &src, &styles[4]);
    assert!(wrapped.lines().count() > plain.lines().count() + 30);
    let long: Vec<&str> = wrapped
        .lines()
        .filter(|l| l.chars().count() > 80)
        .map(str::trim_start)
        .collect();
    let left = [
        "package ",
        "import ",
        "@SuppressWarnings(",
        "println \"a GString",
        "String s = \"a very long string",
        "foo(a, b) // a long trailing comment",
        "implementation group:",
        "names.each { name ->",
        "lines inside a multi-line string",
        "argumentOne ",
        "[alpha, beta, gamma, delta, epsilon]  |",
        "case [alphaAlphaAlpha,",
    ];
    assert_eq!(long.len(), left.len(), "{long:#?}");
    for (l, want) in long.iter().zip(left) {
        assert!(l.starts_with(want), "{l:?} is long, expected {want:?}");
    }
    for line in [
        "        def builder = Foo.builder()\n            .name(\"alpha\")\n",
        "        if (someCondition && otherCondition ||\n            yetAnotherCondition && theLastCondition ||\n            done) {\n",
        "        task(\n            type: Copy,\n",
        "        ) {\n",
        "        return computeTheFirstThing(argumentOne) +\n            computeTheSecondThing(argumentTwo) +\n            more\n",
        "        def other = configuredValueFromTheEnvironment ?:\n            defaultValueFromTheConfigurationFile\n",
    ] {
        assert!(wrapped.contains(line), "{line:?} in\n{wrapped}");
    }
    assert!(
        changed_when_scrambled * 2 >= paths.len(),
        "only {changed_when_scrambled} of {} fixtures change when scrambled",
        paths.len()
    );
}
