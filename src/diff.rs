//! Unified diffs for `--diff`.

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
}
