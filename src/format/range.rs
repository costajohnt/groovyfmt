//! `--lines`: formatting only some lines of a file, and `groovyfmt: off` regions, which
//! `format` leaves as they are by formatting the rest as `--lines` does.

use super::{Outcome, Style, format_rows, lone_cr};
use crate::equiv::whitespace_only;

/// `format`, changing only the lines in `ranges` (1-based, inclusive): the whole file is
/// formatted, since indentation comes from the whole tree, then every line outside the
/// ranges is put back as it was. The result goes through the same token check as `format`.
/// With `max_line_length`, only lines in the ranges are wrapped, and a wrapped line's pieces
/// all take its place.
pub fn format_lines(src: &str, style: &Style, ranges: &[(usize, usize)]) -> Outcome {
    lone_cr(src, |src| lines_of(src, style, ranges))
}

fn lines_of(src: &str, style: &Style, ranges: &[(usize, usize)]) -> Outcome {
    let ranges = &without(ranges, &off_lines(src));
    let out = match format_rows(src, style, ranges) {
        Outcome::Formatted(out) => out,
        other => return other,
    };
    let Some(kept) = keep_outside(src, &out, ranges) else {
        return Outcome::Skipped("token mismatch after formatting");
    };
    if kept == src {
        return Outcome::Unchanged;
    }
    let (before, after) = (crate::parse(src), crate::parse(&kept));
    let same_shape = style.normalize_spacing || style.max_line_length.is_some();
    if !whitespace_only((src, &before), (&kept, &after), same_shape) {
        return Outcome::Skipped("token mismatch after formatting");
    }
    Outcome::Formatted(kept)
}

/// The lines formatting leaves alone: from a `// groovyfmt: off` comment to the next
/// `// groovyfmt: on` (or the end of the file), both comment lines included, 1-based. Either
/// comment form works; the comment must hold nothing else.
pub fn off_lines(src: &str) -> Vec<(usize, usize)> {
    // most files have none: skip the parse
    if !src.contains("groovyfmt: off") {
        return Vec::new();
    }
    let tree = crate::parse(src);
    let (mut out, mut from) = (Vec::new(), None);
    crate::preorder(tree.root_node(), |n| {
        if !n.kind().ends_with("comment") {
            return true;
        }
        let t = crate::text(n, src);
        let body = t
            .strip_prefix("//")
            .or_else(|| t.strip_prefix("/*").and_then(|b| b.strip_suffix("*/")))
            .unwrap_or("")
            .trim();
        match (body, from) {
            ("groovyfmt: off", None) => from = Some(n.start_position().row + 1),
            ("groovyfmt: on", Some(f)) => {
                out.push((f, n.end_position().row + 1));
                from = None;
            }
            _ => {}
        }
        false
    });
    if let Some(f) = from {
        out.push((f, usize::MAX));
    }
    out
}

/// `ranges` with the lines in `off` taken out.
fn without(ranges: &[(usize, usize)], off: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = ranges.to_vec();
    for &(a, b) in off {
        out = out
            .into_iter()
            .flat_map(|(s, e)| {
                let mut parts = Vec::new();
                if s < a {
                    parts.push((s, e.min(a - 1)));
                }
                if e > b {
                    parts.push((s.max(b.saturating_add(1)), e));
                }
                parts
            })
            .collect();
    }
    out
}

/// Whether two lines hold the same text apart from spaces and tabs (and line ending).
fn same_but_spaces(a: &str, b: &str) -> bool {
    let text = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    text(a) == text(b)
}

/// `out` (the formatted `src`) for the lines in `ranges`, `src` for the others. Formatting
/// keeps every non-blank line, in order, changing only its spaces and tabs (the leading and
/// trailing ones, and with `normalize_spacing` those between tokens), except that wrapping
/// splits a line in a range into several, and drops some blank lines, so each source line
/// pairs with the formatted line, or run of lines, holding the same text, or with none.
/// `None` if they don't pair up that way.
///
/// A blank line formatting drops is kept, emptied, so no line is removed: line numbers after
/// a range stay put unless a line in it was wrapped, and running again with the same ranges
/// changes nothing.
fn keep_outside(src: &str, out: &str, ranges: &[(usize, usize)]) -> Option<String> {
    let formatted: Vec<&str> = out.split_inclusive('\n').collect();
    let text = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let mut j = 0;
    let mut s = String::with_capacity(src.len());
    for (i, line) in src.split_inclusive('\n').enumerate() {
        let eol = &line[line.trim_end_matches(['\r', '\n']).len()..];
        let n = i + 1;
        let inside = ranges.iter().any(|&(a, b)| a <= n && n <= b);
        let from = j;
        if formatted.get(j).is_some_and(|f| same_but_spaces(f, line)) {
            j += 1;
        } else if line.trim().is_empty() {
            s.push_str(if inside { eol } else { line });
            continue;
        } else if inside {
            // a wrapped line: the formatted lines whose text runs together into this one's
            let want = text(line);
            let mut got = String::new();
            while got.len() < want.len() {
                let more = text(formatted.get(j)?);
                got.push_str(&more);
                if more.is_empty() || !want.starts_with(&got) {
                    return None;
                }
                j += 1;
            }
        } else {
            return None;
        }
        if inside {
            s.extend(formatted[from..j].iter().copied());
        } else {
            s.push_str(line);
        }
    }
    (j == formatted.len()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_of_a_classic_mac_file() {
        // M4: a lone `\r` ends a line here as in `format`; it was all one line, and the
        // last `\r` came back as `\n`
        let src = "def f() {\rx()\r}\r";
        assert_eq!(
            format_lines(src, &Style::default(), &[(2, 2)]),
            Outcome::Formatted("def f() {\r    x()\r}\r".into())
        );
        assert_eq!(
            format_lines(src, &Style::default(), &[(1, 1)]),
            Outcome::Unchanged
        );
    }

    #[test]
    fn off_comments_keep_their_lines_byte_for_byte() {
        let src = "class A {\ndef f() {\n// groovyfmt: off\nint[][] m = [[1,0],\n             [0,1]]   \n/* groovyfmt: on */\nprintln 1   \n}\n\n\n}\n";
        for spacing in [false, true] {
            let style = Style {
                normalize_spacing: spacing,
                ..Style::default()
            };
            let Outcome::Formatted(out) = crate::format::format(src, &style) else {
                panic!()
            };
            assert_eq!(
                out,
                "class A {\n    def f() {\n// groovyfmt: off\nint[][] m = [[1,0],\n             [0,1]]   \n/* groovyfmt: on */\n        println 1\n    }\n\n\n}\n",
                "{spacing}"
            );
            assert_eq!(
                crate::format::format(&out, &style),
                Outcome::Unchanged,
                "idempotent"
            );
        }
        // an `off` without an `on` runs to the end; one in a string is text
        let src = "def f() {\n  // groovyfmt: off\n      x()\n}\n";
        assert_eq!(
            crate::format::format(src, &Style::default()),
            Outcome::Unchanged
        );
        let src = "def s = '// groovyfmt: off'\n    x()\n";
        assert_eq!(
            crate::format::format(src, &Style::default()),
            Outcome::Formatted("def s = '// groovyfmt: off'\nx()\n".into())
        );
        // --lines inside an off region changes nothing there
        let src = "x()\n// groovyfmt: off\n    y()\n";
        assert_eq!(
            format_lines(src, &Style::default(), &[(3, 3)]),
            Outcome::Unchanged
        );
    }

    #[test]
    fn only_the_given_lines_change() {
        let src = "class A {\ndef f() {\nprintln 1   \n}\n\n\n\n}\n";
        let style = Style::default();
        let Outcome::Formatted(out) = format_lines(src, &style, &[(3, 3)]) else {
            panic!()
        };
        assert_eq!(out, "class A {\ndef f() {\n        println 1\n}\n\n\n\n}\n");
        // stable with the same ranges
        assert_eq!(format_lines(&out, &style, &[(3, 3)]), Outcome::Unchanged);
        // every line: formatting the whole file, except that no line is removed
        let Outcome::Formatted(all) = format_lines(src, &style, &[(1, 100)]) else {
            panic!()
        };
        assert_eq!(
            all,
            "class A {\n    def f() {\n        println 1\n    }\n\n\n\n}\n"
        );
        // blank lines in a range are emptied, never dropped, so later lines keep their
        // numbers: a second range after them still names the same line
        let src = "class A {\ndef f() {\n  \n\t\n\nprintln 1\n}\n}";
        let ranges = [(3, 5), (6, 6)];
        let Outcome::Formatted(out) = format_lines(src, &style, &ranges) else {
            panic!()
        };
        assert_eq!(out, "class A {\ndef f() {\n\n\n\n        println 1\n}\n}");
        assert_eq!(format_lines(&out, &style, &ranges), Outcome::Unchanged);
        assert_eq!(format_lines(src, &style, &[(1, 1)]), Outcome::Unchanged);
    }

    #[test]
    fn only_the_given_lines_are_wrapped() {
        let style = Style {
            max_line_length: Some(40),
            ..Style::default()
        };
        let long = "foo(alpha, beta, gamma, delta, epsilon)";
        let src = format!("def f() {{\n{long}\n  {long}\n    {long}\n}}\n");
        let wrapped = "    foo(\n        alpha,\n        beta,\n        gamma,\n        delta,\n        epsilon\n    )\n";
        // the second line of the body: its pieces take its place, the rest is as it was
        let Outcome::Formatted(out) = format_lines(&src, &style, &[(3, 3)]) else {
            panic!()
        };
        assert_eq!(
            out,
            format!("def f() {{\n{long}\n{wrapped}    {long}\n}}\n")
        );
        // the whole file: as formatting it
        let Outcome::Formatted(all) = format_lines(&src, &style, &[(1, 100)]) else {
            panic!()
        };
        assert_eq!(all, format!("def f() {{\n{0}{0}{0}}}\n", wrapped));
        assert_eq!(
            crate::format::format(&src, &style),
            Outcome::Formatted(all.clone())
        );
        assert_eq!(format_lines(&all, &style, &[(1, 100)]), Outcome::Unchanged);
        // a line in the range that needs no wrapping, beside one that does
        let Outcome::Formatted(out) = format_lines(&src, &style, &[(1, 2)]) else {
            panic!()
        };
        assert_eq!(
            out,
            format!("def f() {{\n{wrapped}  {long}\n    {long}\n}}\n")
        );
        // with spacing too
        let spaced = Style {
            normalize_spacing: true,
            ..style
        };
        let src = "x  = 1\nyy = foo(alpha,beta,gamma,delta,epsilon,zeta)\nz=2\n";
        let Outcome::Formatted(out) = format_lines(src, &spaced, &[(2, 2)]) else {
            panic!()
        };
        assert_eq!(
            out,
            "x  = 1\nyy = foo(\n    alpha,\n    beta,\n    gamma,\n    delta,\n    epsilon,\n    zeta\n)\nz=2\n"
        );
    }

    #[test]
    fn only_the_given_lines_change_with_normalize_spacing() {
        let style = Style {
            normalize_spacing: true,
            ..Style::default()
        };
        // lines pair up across spacing changes; outside the range nothing moves, inside it
        // the `=` column lined up with the line above stays and the rest is respaced
        let src = "x  = f(a,b)\nyy = g(c,d)\nz  =  h(e,f)\n";
        for (range, want) in [
            ((1, 1), "x  = f(a, b)\nyy = g(c,d)\nz  =  h(e,f)\n"),
            ((3, 3), "x  = f(a,b)\nyy = g(c,d)\nz  = h(e, f)\n"),
        ] {
            let Outcome::Formatted(out) = format_lines(src, &style, &[range]) else {
                panic!("{range:?}")
            };
            assert_eq!(out, want, "{range:?}");
            assert_eq!(
                format_lines(&out, &style, &[range]),
                Outcome::Unchanged,
                "{range:?}"
            );
        }
    }
}
