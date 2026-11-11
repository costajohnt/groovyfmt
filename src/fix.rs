//! `--lint --fix`: delete what UnusedImport, DuplicateImport and UnnecessarySemicolon flag
//! (their `Violation::fix` ranges), then prove the result safe, as `format` does its output.
//! The fixed text must parse cleanly; its tokens must be the input's minus exactly the
//! deleted ones; its parse tree must have the input's shape minus the deleted nodes (so a
//! removed `;` can't join two statements, or make a statement another's body); and linting
//! it must find the input's other violations and nothing new. Otherwise the file is left as
//! it was. Removing one `;` can expose another (`a();;`), so this runs a few rounds.

use crate::lint::{Violation, lint_file};
use std::ops::Range;
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug, PartialEq)]
pub enum Fix {
    /// Nothing to fix.
    Nothing,
    /// The fixed text, and the violations fixed.
    Fixed(String, Vec<Violation>),
    /// The file doesn't parse, or a fix failed a check; nothing changes.
    Refused(&'static str),
}

const MAX_ROUNDS: usize = 4;

/// Fix the violations of `src`, the text of `path` (which decides the Jenkins, Gradle and
/// Spock rules), that have a fix and that `wanted` (a rule enabled in the config) accepts.
pub fn fix(path: &Path, src: &str, wanted: impl Fn(&Violation) -> bool) -> Fix {
    let auto = |v: &Violation| v.fix.is_some() && wanted(v);
    let mut text = src.to_string();
    let mut fixed = Vec::new();
    for _ in 0..MAX_ROUNDS {
        let Ok(vs) = lint_file(path, &text) else {
            return Fix::Refused("parse error");
        };
        let (todo, keep): (Vec<_>, Vec<_>) = vs.into_iter().partition(|v| auto(v));
        if todo.is_empty() {
            break;
        }
        let removed = ranges(&todo);
        let out = delete(&text, &removed);
        if let Err(why) = check(path, &text, &out, &removed, keep, &auto) {
            return Fix::Refused(why);
        }
        fixed.extend(todo);
        text = out;
    }
    if fixed.is_empty() {
        Fix::Nothing
    } else {
        Fix::Fixed(text, fixed)
    }
}

/// The ranges to delete, sorted, each inside none of the others (the `;` of a removed
/// import line goes with the line).
fn ranges(todo: &[Violation]) -> Vec<Range<usize>> {
    let mut rs: Vec<Range<usize>> = todo.iter().filter_map(|v| v.fix.clone()).collect();
    rs.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut out: Vec<Range<usize>> = Vec::new();
    for r in rs {
        if out.last().is_none_or(|last| r.start >= last.end) {
            out.push(r);
        } else if let Some(last) = out.last_mut() {
            // overlapping without nesting can't happen (lines and single `;`), but if it
            // does, delete the union; the checks below still decide
            last.end = last.end.max(r.end);
        }
    }
    out
}

fn delete(src: &str, removed: &[Range<usize>]) -> String {
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for r in removed {
        out.push_str(&src[at..r.start]);
        at = r.end;
    }
    out.push_str(&src[at..]);
    out
}

fn inside(n: Node, removed: &[Range<usize>]) -> bool {
    removed
        .iter()
        .any(|r| r.start <= n.start_byte() && n.end_byte() <= r.end)
}

fn check(
    path: &Path,
    src: &str,
    out: &str,
    removed: &[Range<usize>],
    keep: Vec<Violation>,
    auto: &impl Fn(&Violation) -> bool,
) -> Result<(), &'static str> {
    let (before, after) = (crate::parse(src), crate::parse(out));
    if after.root_node().has_error() {
        return Err("the fixed file would not parse");
    }
    if leaves(src, &before, removed) != leaves(out, &after, &[]) {
        return Err("tokens other than the removed ones would change");
    }
    if shape(&before, removed) != shape(&after, &[]) {
        return Err("the parse tree would change shape");
    }
    // the other violations stay, except one inside a removed line (a `;` whose rule is off);
    // new fixable ones are left for the next round
    let key = |v: &Violation| (v.rule, v.message.clone());
    let mut want: Vec<_> = keep
        .iter()
        .filter(|v| {
            !v.fix
                .as_ref()
                .is_some_and(|f| removed.iter().any(|r| r.start <= f.start && f.end <= r.end))
        })
        .map(key)
        .collect();
    let mut got: Vec<_> = lint_file(path, out)
        .map_err(|_| "the fixed file would not parse")?
        .iter()
        .filter(|v| !auto(v))
        .map(key)
        .collect();
    want.sort();
    got.sort();
    if want != got {
        return Err("other lint violations would change");
    }
    Ok(())
}

/// Leaf tokens outside `removed`, as (kind, text).
fn leaves<'a>(src: &'a str, tree: &Tree, removed: &[Range<usize>]) -> Vec<(u16, &'a str)> {
    let mut v = Vec::new();
    crate::preorder(tree.root_node(), |n| {
        if inside(n, removed) {
            return false;
        }
        if n.child_count() == 0 {
            v.push((n.kind_id(), crate::text(n, src)));
        }
        true
    });
    v
}

/// Named nodes outside `removed`, as (kind, field, depth) in document order.
fn shape(tree: &Tree, removed: &[Range<usize>]) -> Vec<(u16, Option<u16>, usize)> {
    let mut v = Vec::new();
    let mut c = tree.walk();
    let mut depth = 0;
    loop {
        let n = c.node();
        let skip = inside(n, removed);
        if !skip && n.is_named() {
            v.push((n.kind_id(), c.field_id().map(|f| f.get()), depth));
        }
        if !skip && c.goto_first_child() {
            depth += 1;
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return v;
            }
            depth -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::lint;

    fn fixed(src: &str) -> Fix {
        fix(Path::new("A.groovy"), src, |_| true)
    }

    fn text(src: &str) -> String {
        match fixed(src) {
            Fix::Fixed(t, _) => t,
            other => panic!("{src:?}: {other:?}"),
        }
    }

    #[test]
    fn removes_unused_and_duplicate_import_lines_and_semicolons() {
        let src = "import a.Used\nimport a.Unused\nimport a.Used\nimport b.X;\n\nUsed u = new Used();\nprintln u;\n";
        assert_eq!(
            text(src),
            "import a.Used\n\nUsed u = new Used()\nprintln u\n"
        );
        assert_eq!(fixed(&text(src)), Fix::Nothing, "idempotent");
    }

    #[test]
    fn imports_used_anywhere_lint_counts_a_use_stay() {
        for (import, usage) in [
            ("import a.Foo", "println \"$Foo\""),
            ("import a.Foo", "println \"${Foo.X}\""),
            ("import a.Foo", "/** see {@link Foo} */\nclass X {}"),
            ("import a.Foo", "@Foo\nclass X {}"),
            ("import static a.B.go", "go()"),
            ("import a.B as C", "C.go()"),
            ("import a.*", "x()"),
            ("import static a.B.*", "x()"),
        ] {
            let src = format!("{import}\n{usage}\n");
            assert_eq!(fixed(&src), Fix::Nothing, "{src:?}");
        }
        // unused static, aliased and duplicate star imports go
        assert_eq!(text("import static a.B.go\nx()\n"), "x()\n");
        assert_eq!(text("import a.B as C\nB.go()\n"), "B.go()\n");
        assert_eq!(text("import a.*\nimport a.*\nx()\n"), "import a.*\nx()\n");
        // a mention in a string is not a use
        assert_eq!(text("import a.Foo\nprintln 'a Foo'\n"), "println 'a Foo'\n");
        assert_eq!(text("import a.Foo\nprintln 'Foo'\n"), "println 'Foo'\n");
    }

    #[test]
    fn leaves_lines_holding_anything_else() {
        // two imports on a line, a comment after one: reported but not fixable
        let src = "import a.B; import c.D\nimport e.F // why\nx = 1\n";
        let vs = lint(src).unwrap();
        assert!(
            vs.iter()
                .all(|v| v.rule != "UnusedImport" || v.fix.is_none()),
            "{vs:?}"
        );
        // the `;` between the two imports is not at a line end
        assert_eq!(fixed(src), Fix::Nothing);
    }

    #[test]
    fn line_endings_and_a_missing_final_newline() {
        assert_eq!(text("import a.B\r\nx = 1;\r\n"), "x = 1\r\n");
        assert_eq!(text("x = 1\nimport a.B"), "x = 1\n");
        assert_eq!(text("x = 1;"), "x = 1");
    }

    #[test]
    fn semicolons_that_are_needed_stay() {
        for src in [
            "a(); b()\n",
            "for (int i = 0; i < 3; i++) { f(i) }\n",
            "for (;;) { f() }\n",
            "while (it.next());\nf()\n",
            "f();\n{ -> g() }.call()\n",
        ] {
            assert_eq!(fixed(src), Fix::Nothing, "{src:?}");
        }
        // a `(` or `[` line after it starts a statement of its own
        assert_eq!(
            text("b();\n(c).d()\ne();\n[1].f()\n"),
            "b()\n(c).d()\ne()\n[1].f()\n"
        );
        // removing one exposes the next
        assert_eq!(text("a();;\n"), "a()\n");
    }

    #[test]
    fn unparseable_files_are_never_fixed() {
        assert_eq!(
            fixed("import a.B;\ndef x = {\n"),
            Fix::Refused("parse error")
        );
        // the grammar doesn't parse an empty `for (;;)` or `if` body, so those aren't either
        for src in [
            "for (;;);\nf()\n",
            "if (c);\nf()\n",
            "for (x in xs);\nf()\n",
        ] {
            assert_eq!(fixed(src), Fix::Refused("parse error"), "{src:?}");
        }
    }

    #[test]
    fn only_wanted_rules_are_fixed() {
        let src = "import a.B;\nx = 1;\n";
        let semis = |v: &Violation| v.rule == "UnnecessarySemicolon";
        let Fix::Fixed(t, f) = fix(Path::new("A.groovy"), src, semis) else {
            panic!()
        };
        assert_eq!((t.as_str(), f.len()), ("import a.B\nx = 1\n", 2));
        // the import line goes with its `;` even when UnnecessarySemicolon is off
        let imports = |v: &Violation| v.rule == "UnusedImport";
        let Fix::Fixed(t, f) = fix(Path::new("A.groovy"), src, imports) else {
            panic!()
        };
        assert_eq!(t, "x = 1;\n");
        assert_eq!(f.len(), 1);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // one range deleted is the common case
    fn the_checks_reject_a_change_of_shape() {
        // without the empty-body `;` (which lint never flags), `a()` would become the body:
        // same tokens, different tree
        assert_eq!(
            check(
                Path::new("A.groovy"),
                "while (x);\na()\n",
                "while (x)\na()\n",
                &[9..10],
                Vec::new(),
                &|_| true
            ),
            Err("the parse tree would change shape")
        );
        // a change of tokens
        assert!(
            check(
                Path::new("A.groovy"),
                "x = 1;\n",
                "x = 2\n",
                &[5..6],
                Vec::new(),
                &|_| true
            )
            .is_err()
        );
        assert!(
            check(
                Path::new("A.groovy"),
                "x = 1;\n",
                "x = 1\n",
                &[5..6],
                Vec::new(),
                &|_| true
            )
            .is_ok()
        );
        assert_eq!(
            check(
                Path::new("A.groovy"),
                "x = 1;\ny = 2\n",
                "x = 1\ny = 2 +\n",
                &[5..6],
                Vec::new(),
                &|_| true
            ),
            Err("the fixed file would not parse")
        );
    }
}
