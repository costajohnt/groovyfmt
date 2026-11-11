//! Unified diffs for `--diff`, and `--lines`: formatting only some lines of a file.

use crate::format::{Outcome, Style, format_whole};

/// One aligned line: in the old text only, the new only, or both (unchanged).
#[derive(Clone, Copy, PartialEq)]
enum Op {
    Old(usize),
    New(usize),
    Both(usize, usize),
}

/// Lines compared ignoring whitespace and `;`: what formatting and `--fix` change.
fn key(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_whitespace() && *c != ';')
        .collect()
}

/// How far ahead to look for the next pair of matching lines.
const LOOKAHEAD: usize = 64;

/// A line-by-line alignment of `a` with `b`. Formatting and `--fix` keep the order of lines
/// and change them only in whitespace and `;`, or drop them (blank lines, import lines), so
/// lines are paired by `key` with a short lookahead rather than a general diff. Any
/// alignment gives a correct diff; this one gives a readable one for those changes in
/// linear time.
fn align(a: &[&str], b: &[&str]) -> Vec<Op> {
    let (ka, kb): (Vec<String>, Vec<String>) = (
        a.iter().map(|l| key(l)).collect(),
        b.iter().map(|l| key(l)).collect(),
    );
    let (mut i, mut j, mut ops) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            ops.push(Op::Both(i, j));
            (i, j) = (i + 1, j + 1);
            continue;
        }
        if i < a.len() && j < b.len() && ka[i] == kb[j] {
            ops.extend([Op::Old(i), Op::New(j)]);
            (i, j) = (i + 1, j + 1);
            continue;
        }
        let ahead = |k: &[String], from: usize, other: Option<&String>| {
            (1..=LOOKAHEAD).find(|&n| other.is_some() && k.get(from + n) == other)
        };
        match (ahead(&ka, i, kb.get(j)), ahead(&kb, j, ka.get(i))) {
            (Some(d), ins) if ins.is_none_or(|n| d <= n) => {
                ops.extend((i..i + d).map(Op::Old));
                i += d;
            }
            (_, Some(n)) => {
                ops.extend((j..j + n).map(Op::New));
                j += n;
            }
            _ => {
                if i < a.len() {
                    ops.push(Op::Old(i));
                    i += 1;
                }
                if j < b.len() {
                    ops.push(Op::New(j));
                    j += 1;
                }
            }
        }
    }
    ops
}

const CONTEXT: usize = 3;

/// A unified diff from `old` to `new`, headed `a/<path>` and `b/<path>` as git writes them,
/// so `git apply` and `patch -p1` take it; empty when they are equal.
pub fn unified(path: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (
        old.split_inclusive('\n').collect(),
        new.split_inclusive('\n').collect(),
    );
    // each change block's removed lines first, then its added lines
    let mut rows: Vec<Op> = Vec::new();
    let mut block: (Vec<Op>, Vec<Op>) = (Vec::new(), Vec::new());
    for op in align(&a, &b) {
        match op {
            Op::Old(_) => block.0.push(op),
            Op::New(_) => block.1.push(op),
            Op::Both(..) => {
                rows.append(&mut block.0);
                rows.append(&mut block.1);
                rows.push(op);
            }
        }
    }
    rows.append(&mut block.0);
    rows.append(&mut block.1);
    // the old and new lines before each row
    let mut before = Vec::with_capacity(rows.len());
    let (mut o, mut n) = (0, 0);
    for op in &rows {
        before.push((o, n));
        match op {
            Op::Old(_) => o += 1,
            Op::New(_) => n += 1,
            Op::Both(..) => (o, n) = (o + 1, n + 1),
        }
    }
    let changed: Vec<usize> = (0..rows.len())
        .filter(|&r| !matches!(rows[r], Op::Both(..)))
        .collect();
    let mut s = format!("--- a/{path}\n+++ b/{path}\n");
    let mut k = 0;
    while k < changed.len() {
        let first = changed[k];
        let mut last = first;
        while k + 1 < changed.len() && changed[k + 1] - last <= 2 * CONTEXT + 1 {
            k += 1;
            last = changed[k];
        }
        k += 1;
        let (lo, hi) = (
            first.saturating_sub(CONTEXT),
            (last + CONTEXT).min(rows.len() - 1),
        );
        let span = &rows[lo..=hi];
        let olds = span.iter().filter(|r| !matches!(r, Op::New(_))).count();
        let news = span.iter().filter(|r| !matches!(r, Op::Old(_))).count();
        let (o0, n0) = before[lo];
        // an empty side is numbered by the line before it
        s += &format!(
            "@@ -{},{olds} +{},{news} @@\n",
            o0 + usize::from(olds > 0),
            n0 + usize::from(news > 0)
        );
        for r in span {
            let (tag, line) = match *r {
                Op::Old(i) => ('-', a[i]),
                Op::New(j) => ('+', b[j]),
                Op::Both(i, _) => (' ', a[i]),
            };
            s.push(tag);
            s.push_str(line);
            if !line.ends_with('\n') {
                s.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    s
}

/// `format`, changing only the lines in `ranges` (1-based, inclusive): the whole file is
/// formatted, since indentation comes from the whole tree, then every line outside the
/// ranges is put back as it was. The result goes through the same token check as `format`.
pub fn format_lines(src: &str, style: &Style, ranges: &[(usize, usize)]) -> Outcome {
    let ranges = &without(ranges, &off_lines(src));
    let out = match format_whole(src, style) {
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
    if after.root_node().has_error()
        || crate::format::tokens(src, &before) != crate::format::tokens(&kept, &after)
        || (style.normalize_spacing
            && crate::format::shape(&before) != crate::format::shape(&after))
    {
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
/// trailing ones, and with `normalize_spacing` those between tokens),
/// and drops some blank lines, so each source line pairs with the formatted line holding the
/// same text, or with none. `None` if they don't pair up that way.
///
/// A blank line formatting drops is kept, emptied, so no line is added or removed: line
/// numbers after a range stay put, and running again with the same ranges changes nothing.
fn keep_outside(src: &str, out: &str, ranges: &[(usize, usize)]) -> Option<String> {
    let mut formatted = out.split_inclusive('\n').peekable();
    let mut s = String::with_capacity(src.len());
    for (i, line) in src.split_inclusive('\n').enumerate() {
        let eol = &line[line.trim_end_matches(['\r', '\n']).len()..];
        let paired = formatted
            .next_if(|f| same_but_spaces(f, line))
            .or_else(|| line.trim().is_empty().then_some(eol))?;
        let n = i + 1;
        s.push_str(if ranges.iter().any(|&(a, b)| a <= n && n <= b) {
            paired
        } else {
            line
        });
    }
    formatted.next().is_none().then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_name_both_sides_and_number_hunks() {
        let old = "a\nb\nc\n  d\ne\nf\ng\nh\ni\nj\nk\nl\n  m\n";
        let new = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\n";
        assert_eq!(
            unified("x.groovy", old, new),
            "--- a/x.groovy\n+++ b/x.groovy\n@@ -1,7 +1,7 @@\n a\n b\n c\n-  d\n+d\n e\n f\n g\n@@ -10,4 +10,4 @@\n j\n k\n l\n-  m\n+m\n"
        );
        assert_eq!(unified("x", "a\n", "a\n"), "");
    }

    #[test]
    fn dropped_lines_and_missing_final_newlines() {
        assert_eq!(
            unified("x", "import a.B\nx = 1;\n\n\ny", "x = 1\n\ny\n"),
            "--- a/x\n+++ b/x\n@@ -1,5 +1,3 @@\n-import a.B\n-x = 1;\n+x = 1\n \n-\n-y\n\\ No newline at end of file\n+y\n"
        );
        assert_eq!(
            unified("x", "a\n", ""),
            "--- a/x\n+++ b/x\n@@ -1,1 +0,0 @@\n-a\n"
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
