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

#[test]
fn fixtures_format_whitespace_only_and_idempotently() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
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
    assert!(
        changed_when_scrambled * 2 >= paths.len(),
        "only {changed_when_scrambled} of {} fixtures change when scrambled",
        paths.len()
    );
}
