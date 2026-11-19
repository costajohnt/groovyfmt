//! `--lint --fix`: apply the edits of the violations that have one (`Violation::fix`):
//! delete what UnusedImport, DuplicateImport, ImportFromSamePackage, UnnecessarySemicolon
//! and Groovy24EmptyElement flag, single-quote what UnnecessaryGString flags, and rewrite
//! what the fixable Gradle rules flag. Then prove the
//! result safe, as `format` does its output. The fixed text must parse cleanly; its tokens
//! outside the edited ranges must be the input's outside them; its parse tree outside them
//! must have the input's shape (so a removed `;` can't join two statements, or make a
//! statement another's body, and a rewritten statement can't take in its neighbor); and
//! linting it must find the input's other violations and nothing new. Otherwise the file is
//! left as it was. A rewritten dependency must also parse as the same call. One fix can
//! expose another (`a();;`), and of two overlapping edits only the first is made in a round,
//! so this runs rounds until one finds nothing to fix. A fix failing the checks on its own is
//! dropped, its report left standing, and the others are made without it.

use crate::lint::{Context, Edit, Violation, lint_opt_in};
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

/// Rounds before giving up on a file whose fixes keep exposing more: one per level of
/// nesting (`task a << { task b << { } }`), and real files need a few.
const MAX_ROUNDS: usize = 64;

/// Rules whose fix rewrites a dependency statement, which must parse as the same call.
const CALL_REWRITES: &[&str] = &["GradleDependencyParentheses", "GradleDependencyTuple"];
/// The rule whose fix turns a call into an assignment of the same value to its property.
const ASSIGNMENT_REWRITE: &str = "GradleSpaceAssignment";

/// Fix the violations of `src`, the text of `path` (which decides the Jenkins, Gradle and
/// Spock rules), that have a fix and that `wanted` (a rule enabled in the config, and not
/// in the baseline) accepts; it is also given the text the violation is in. `compile` is
/// the config's `gradle_compile_replacement`, which makes `compile` dependencies fixable.
/// `cx` gives the Gradle files around a build script (`Projects::gradle_context`); every
/// rule is checked, without Spock bases from the project.
pub fn fix(
    path: &Path,
    src: &str,
    compile: Option<&str>,
    cx: &Context,
    wanted: impl Fn(&Violation, &str) -> bool,
) -> Fix {
    let mut text = src.to_string();
    let mut fixed = Vec::new();
    // why a fix was dropped, reported when no other fix is made
    let mut dropped = None;
    for n in 0..=MAX_ROUNDS {
        let Ok(vs) = lint_opt_in(path, &text, compile, cx) else {
            return Fix::Refused("parse error");
        };
        let (todo, keep): (Vec<_>, Vec<_>) = vs
            .into_iter()
            .partition(|v| v.fix.is_some() && wanted(v, &text));
        if todo.is_empty() {
            break;
        }
        if n == MAX_ROUNDS {
            return Fix::Refused("fixes still exposed more fixes after 64 rounds; nothing written");
        }
        match round(path, &text, todo, &keep, compile, cx, &wanted, &mut dropped) {
            Err(why) => return Fix::Refused(why),
            Ok(None) => break,
            Ok(Some((out, done))) => {
                fixed.extend(done);
                text = out;
            }
        }
    }
    match (fixed.is_empty(), dropped) {
        (false, _) => Fix::Fixed(text, fixed),
        (true, Some(why)) => Fix::Refused(why),
        (true, None) => Fix::Nothing,
    }
}

/// One round: the text with `todo`'s edits made, and the violations that fixes. An edit that
/// fails the checks on its own is dropped (`dropped` gets why, the first time; found by
/// `bisect`), and the rest are tried again; `None` when all are dropped. An error when the
/// edits fail together but none alone.
#[allow(clippy::too_many_arguments)] // `fix`'s state, passed down
fn round(
    path: &Path,
    text: &str,
    mut todo: Vec<Violation>,
    keep: &[Violation],
    compile: Option<&str>,
    cx: &Context,
    wanted: &impl Fn(&Violation, &str) -> bool,
    dropped: &mut Option<&'static str>,
) -> Result<Option<(String, Vec<Violation>)>, &'static str> {
    let attempt = |es: &[Edit], calls: &[&str]| {
        let (out, inserted) = apply(text, es);
        let auto = |v: &Violation| v.fix.is_some() && wanted(v, &out);
        check(
            path, text, &out, es, &inserted, calls, keep, compile, cx, &auto,
        )
        .map(|()| out)
    };
    while !todo.is_empty() {
        let (made, done) = edits(&todo);
        let es: Vec<Edit> = made.iter().filter_map(|&i| todo[i].fix.clone()).collect();
        let calls: Vec<&str> = made.iter().map(|&i| todo[i].rule).collect();
        match attempt(&es, &calls) {
            Ok(out) => {
                let done = todo
                    .into_iter()
                    .zip(done)
                    .filter(|(_, d)| *d)
                    .map(|(v, _)| v)
                    .collect();
                return Ok(Some((out, done)));
            }
            Err(why) => {
                let mut bad = Vec::new();
                bisect(&attempt, &es, &calls, 0, &mut bad);
                let bad: Vec<usize> = bad.into_iter().map(|k| made[k]).collect();
                if bad.is_empty() {
                    return Err(why);
                }
                dropped.get_or_insert(why);
                let mut i = 0;
                todo.retain(|_| {
                    i += 1;
                    !bad.contains(&(i - 1))
                });
            }
        }
    }
    Ok(None)
}

/// Given edits that fail the checks together, add to `bad` (offset by `at`) each that fails
/// them alone: halves are checked and only failing ones split again, so k bad edits of n
/// take O(k log n) checks, not n. A set failing while both its halves pass fails only in
/// combination, and its edits are not dropped.
// ponytail: an edit failing alone inside a passing half is kept; the round's final check
// covers the whole set anyway, and a set left failing with none found refuses the file
fn bisect<T>(
    attempt: &impl Fn(&[Edit], &[&str]) -> Result<T, &'static str>,
    es: &[Edit],
    calls: &[&str],
    at: usize,
    bad: &mut Vec<usize>,
) {
    if es.len() == 1 {
        bad.push(at);
        return;
    }
    let mid = es.len() / 2;
    for (lo, hi) in [(0, mid), (mid, es.len())] {
        if attempt(&es[lo..hi], &calls[lo..hi]).is_err() {
            bisect(attempt, &es[lo..hi], &calls[lo..hi], at + lo, bad);
        }
    }
}

/// The violations whose edits to make this round (sorted, none overlapping another), and
/// for each violation whether this round fixes it: its edit is made, or lies in a range one
/// made deletes (the `;` of a removed import line goes with the line). An edit overlapping
/// one made otherwise waits for the next round, which lints the text again.
fn edits(todo: &[Violation]) -> (Vec<usize>, Vec<bool>) {
    let mut order: Vec<usize> = (0..todo.len()).collect();
    let range = |i: usize| todo[i].fix.as_ref().map_or(0..0, |e| e.range.clone());
    order.sort_by_key(|&i| (range(i).start, std::cmp::Reverse(range(i).end)));
    let mut out: Vec<usize> = Vec::new();
    let mut done = vec![false; todo.len()];
    for i in order {
        let Some(e) = &todo[i].fix else {
            continue;
        };
        // the same edit for several violations (GradleRemovedApi's import for each use of
        // a name) is made once, and fixes them all
        if out.iter().any(|&l| todo[l].fix.as_ref() == Some(e)) {
            done[i] = true;
            continue;
        }
        match out.last().and_then(|&l| todo[l].fix.as_ref()) {
            Some(last) if e.range.start < last.range.end => {
                done[i] = last.with.is_empty() && e.range.end <= last.range.end;
            }
            _ => {
                out.push(i);
                done[i] = true;
            }
        }
    }
    (out, done)
}

/// The text with `edits` made, and where each edit's new text is in it.
fn apply(src: &str, edits: &[Edit]) -> (String, Vec<Range<usize>>) {
    let mut out = String::with_capacity(src.len());
    let mut inserted = Vec::new();
    let mut at = 0;
    for e in edits {
        out.push_str(&src[at..e.range.start]);
        inserted.push(out.len()..out.len() + e.with.len());
        out.push_str(&e.with);
        at = e.range.end;
    }
    out.push_str(&src[at..]);
    (out, inserted)
}

fn inside(n: Node, removed: &[Range<usize>]) -> bool {
    removed
        .iter()
        .any(|r| r.start <= n.start_byte() && n.end_byte() <= r.end)
}

#[cfg(test)]
thread_local! {
    /// Checks run on this thread, so a test can bound the work of a round.
    static CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `calls` is each edit's rule, which says whether it rewrites a dependency statement
/// (`CALL_REWRITES`) or turns a call into an assignment (`ASSIGNMENT_REWRITE`).
#[allow(clippy::too_many_arguments)] // the two texts and the edits between them
fn check(
    path: &Path,
    src: &str,
    out: &str,
    edits: &[Edit],
    inserted: &[Range<usize>],
    calls: &[&str],
    keep: &[Violation],
    compile: Option<&str>,
    cx: &Context,
    auto: &impl Fn(&Violation) -> bool,
) -> Result<(), &'static str> {
    #[cfg(test)]
    CHECKS.with(|c| c.set(c.get() + 1));
    let removed: Vec<Range<usize>> = edits.iter().map(|e| e.range.clone()).collect();
    let (before, after) = (crate::parse(src), crate::parse(out));
    if after.root_node().has_error() {
        return Err("the fixed file would not parse");
    }
    let (b, a) = (before.root_node(), after.root_node());
    for ((e, ins), rule) in edits.iter().zip(inserted).zip(calls) {
        if CALL_REWRITES.contains(rule) && !crate::lint::same_call(src, b, &e.range, out, a, ins) {
            return Err("a rewritten dependency would parse as a different call");
        }
        if *rule == ASSIGNMENT_REWRITE
            && !crate::lint::same_assignment(src, b, &e.range, out, a, ins)
        {
            return Err("a space assignment would not become the same assignment");
        }
    }
    // A net under UnusedImport's own idea of a use (#165 slipped past it, and past the
    // checks below, which compare trees, not names): a removed import's name must not be
    // left as an identifier anywhere but the name or alias of another import, nor a static
    // accessor import's property name (`SCOPES` for `getSCOPES`, #181).
    let gone: Vec<String> = edits
        .iter()
        .zip(calls)
        .filter(|(_, r)| **r == "UnusedImport")
        .flat_map(|(e, _)| import_symbols(src, b, &e.range))
        .collect();
    // Hubitat appends an `#include`d library to the file, and it can use any import (#192)
    if crate::lint::hubitat_include(src, b)
        && edits
            .iter()
            .any(|e| !import_symbols(src, b, &e.range).is_empty())
    {
        return Err("an #include'd library can use any import of the file");
    }
    if !gone.is_empty()
        && identifiers(out, a)
            .iter()
            .any(|t| gone.iter().any(|g| g == t))
    {
        return Err("a removed import's name is still used");
    }
    if leaves(src, &before, &removed) != leaves(out, &after, inserted) {
        return Err("tokens other than the edited ones would change");
    }
    if shape(&before, &removed) != shape(&after, inserted) {
        return Err("the parse tree would change shape");
    }
    // the grammar reads `FOO` then `bar()` as two statements, as Groovy 2 does, but Groovy 3+
    // reads one method declaration (#211); the trees above can't see that
    if joined(src, b) < joined(out, a) {
        return Err("two statements would join into a method declaration on Groovy 3+");
    }
    // the other violations stay, except one inside an edited range (a `;` whose rule is off,
    // an unused import that is also unnecessary, a dynamic version in a rewritten
    // dependency), whose counterpart in the new text isn't compared either; new fixable ones
    // are left for the next round. One whose span holds a replacement is compared by rule
    // alone, as its message can quote the replaced code (`"a" == "b"` for a comparison whose
    // strings are single-quoted, or a dependency's suggested string with the new quotes).
    let within = |rs: &[Range<usize>], x: &Range<usize>| {
        rs.iter().any(|r| r.start <= x.start && x.end <= r.end)
    };
    let key = |v: &Violation, replaced: &[Range<usize>]| {
        let quotes = replaced
            .iter()
            .any(|r| within(std::slice::from_ref(&v.span), r));
        (v.rule, (!quotes).then(|| v.message.clone()))
    };
    let replaced: Vec<Range<usize>> = edits
        .iter()
        .filter(|e| !e.with.is_empty())
        .map(|e| e.range.clone())
        .collect();
    let mut want: Vec<_> = keep
        .iter()
        .filter(|v| {
            !within(&removed, &v.span)
                && !v.fix.as_ref().is_some_and(|e| within(&removed, &e.range))
        })
        .map(|v| key(v, &replaced))
        .collect();
    let new: Vec<Range<usize>> = inserted.iter().filter(|r| !r.is_empty()).cloned().collect();
    let vs = lint_opt_in(path, out, compile, cx).map_err(|_| "the fixed file would not parse")?;
    let mut got: Vec<_> = vs
        .iter()
        .filter(|v| !auto(v) && !within(&new, &v.span))
        .map(|v| key(v, &new))
        .collect();
    want.sort();
    got.sort();
    if want != got {
        return Err("other lint violations would change");
    }
    Ok(())
}

/// The names (alias, or the last part, and a static accessor's property names) of the
/// imports inside `range`.
fn import_symbols(src: &str, root: Node, range: &Range<usize>) -> Vec<String> {
    let mut v = Vec::new();
    crate::preorder(root, |n| {
        if n.end_byte() <= range.start || n.start_byte() >= range.end {
            return false;
        }
        if n.kind() != "import_declaration" {
            return true;
        }
        if range.start <= n.start_byte() && n.end_byte() <= range.end {
            v.push(crate::lint::import_name(n, src).1.to_string());
            v.extend(crate::lint::import_property_names(n, src));
        }
        false
    });
    v
}

/// The identifiers of `src` (`"$Name"` included), except imports' names and aliases.
fn identifiers<'a>(src: &'a str, root: Node) -> Vec<&'a str> {
    let mut v = Vec::new();
    crate::preorder(root, |n| {
        let field = |f| n.parent().and_then(|p| p.child_by_field_name(f)) == Some(n);
        if n.parent().is_some_and(|p| p.kind() == "import_declaration")
            && (field("name") || field("alias"))
        {
            return false;
        }
        match n.kind() {
            "identifier" | "type_identifier" => v.push(crate::text(n, src)),
            // `"$Café.v"`: the grammar's `$name` is ASCII only (#192)
            "string_literal" if !crate::text(n, src).starts_with('\'') => {
                crate::lint::dollar_names(crate::text(n, src), &mut v);
            }
            "gstring_dollar_interpolation" => {
                v.extend(
                    crate::text(n, src)
                        .trim_start_matches('$')
                        .split('.')
                        .next(),
                );
            }
            _ => {}
        }
        true
    });
    v
}

/// Adjacent top-level statements with no `;` between that Groovy 3+ joins into a method
/// declaration (`crate::lint::joins_declaration`).
fn joined(src: &str, root: Node) -> usize {
    let mut c = root.walk();
    let stmts: Vec<Node> = root
        .children(&mut c)
        .filter(|n| !n.kind().contains("comment"))
        .collect();
    stmts
        .windows(2)
        .filter(|w| crate::lint::joins_declaration(src, w[0], w[1]))
        .count()
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
    use crate::lint::{lint, lint_file};

    fn fixed(src: &str) -> Fix {
        fix(
            Path::new("A.groovy"),
            src,
            None,
            &Context::new(|_| None),
            |_, _| true,
        )
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
        // #171: used, however spaced around the dots; unused ones still go
        for import in [
            "import java.util.concurrent. TimeUnit",
            "import java.util.concurrent . TimeUnit",
            "import java.util.concurrent.\n    TimeUnit",
            "import java.util.concurrent.\tTimeUnit",
        ] {
            let src = format!("{import}\nTimeUnit t = TimeUnit.SECONDS\n");
            assert_eq!(fixed(&src), Fix::Nothing, "{src:?}");
        }
        let src = "import static java.util.concurrent.TimeUnit. SECONDS\nprintln SECONDS\n";
        assert_eq!(fixed(src), Fix::Nothing);
        assert_eq!(text("import a. Foo\nx()\n"), "x()\n");
        assert_eq!(
            text("import a. Foo\nimport a.Foo\nFoo f\n"),
            "import a. Foo\nFoo f\n"
        );
    }

    #[test]
    fn an_import_whose_name_is_still_used_is_never_removed() {
        // #165: the lint missed `@Gen` on a later import; whatever the lint thinks, removing
        // an import whose name is left as an identifier fails the checks
        let net = |src: &str, line: &str| {
            let at = src.find(line).unwrap();
            let e = Edit {
                range: at..at + line.len() + 1,
                with: String::new(),
            };
            let (out, ins) = apply(src, std::slice::from_ref(&e));
            check(
                Path::new("A.groovy"),
                src,
                &out,
                &[e],
                &ins,
                &["UnusedImport"],
                &[],
                None,
                &Context::new(|_| None),
                &|_| true,
            )
        };
        let gone = Err("a removed import's name is still used");
        for (src, line) in [
            ("import a.Gen\n@Gen\nimport b.T\nT.x()\n", "import a.Gen"),
            (
                "import a.Gen\n@Gen import static b.T.x\nx()\n",
                "import a.Gen",
            ),
            ("import a.B as C\nC.go()\n", "import a.B as C"),
            (
                "import static a.B.go\ndef f(go) {}\n",
                "import static a.B.go",
            ),
            ("import a.Foo\nprintln \"$Foo\"\n", "import a.Foo"),
            ("import a.Foo\ndef x = (Foo) y\n", "import a.Foo"),
            // #171: the name as Groovy reads it, without the spaces around its dots
            ("import a. Foo\nFoo f = null\n", "import a. Foo"),
            ("import a .Foo\nFoo f = null\n", "import a .Foo"),
            ("import a.\n  Foo\nFoo f = null\n", "import a.\n  Foo"),
            ("import a./*c*/Foo\nFoo f = null\n", "import a./*c*/Foo"),
            (
                "import static a.B. SECONDS\nprintln SECONDS\n",
                "import static a.B. SECONDS",
            ),
            // #181: a static accessor import's property, in each form Groovy reads
            (
                "import static a.B.getSCOPES\nprintln SCOPES\n",
                "import static a.B.getSCOPES",
            ),
            (
                "import static a.B.getUrl\nprintln url\n",
                "import static a.B.getUrl",
            ),
            (
                "import static a.B.getUrl\nprintln Url\n",
                "import static a.B.getUrl",
            ),
            (
                "import static a.B.isACTIVE\nprintln ACTIVE\n",
                "import static a.B.isACTIVE",
            ),
            (
                "import static a.B.setURLX\nURLX = 1\n",
                "import static a.B.setURLX",
            ),
            (
                "import static a.B.get_foo\nprintln _foo\n",
                "import static a.B.get_foo",
            ),
            (
                "import static a.B.getFoo as getBar\nprintln bar\n",
                "import static a.B.getFoo as getBar",
            ),
            // #192: the grammar's `$name` is ASCII only, the net reads the whole string
            ("import a.B as Ä\nprintln \"$Ä.X\"\n", "import a.B as Ä"),
            ("import a.Café\nprintln \"$Café.v\"\n", "import a.Café"),
            ("import a.Café\nprintln(/$Café/)\n", "import a.Café"),
            (
                "import static a.B.getRuntime as getRT\nprintln RT\n",
                "import static a.B.getRuntime as getRT",
            ),
        ] {
            assert_eq!(net(src, line), gone, "{src:?}");
        }
        // #192: a Hubitat library `#include`d at the end can use any import
        assert_eq!(
            net(
                "import javax.crypto.Cipher\nmetadata { }\n#include tuya.tuyaProtocols\n",
                "import javax.crypto.Cipher"
            ),
            Err("an #include'd library can use any import of the file")
        );
        // in a string, a comment or another import's name: not a use
        for src in [
            "import a.Foo\nprintln 'Foo' // Foo\n",
            "import a.Foo\nimport b.Foo\nx()\n",
            "import a.Foo\nimport b.Bar as Foo\nx()\n",
        ] {
            assert_eq!(net(src, "import a.Foo"), Ok(()), "{src:?}");
        }
        // a non-static import has no property names
        assert_eq!(net("import a.getFoo\nfoo()\n", "import a.getFoo"), Ok(()));
    }

    #[test]
    fn imports_whose_removal_changes_a_binding_stay() {
        // #192. Unicode names used only through `$name` in each GString kind
        for src in [
            "import a.B as Ä\nprintln \"$Ä.SECONDS\"\n",
            "import a.Café\nprintln \"$Café.v\"\n",
            "import a.Café\nprintln \"\"\"$Café.v\"\"\"\n",
            "import a.Café\nprintln(/$Café.v/)\n",
            "import a.Café\nprintln($/$Café.v/$)\n",
            "import static a.B.SECONDS as Séc\nprintln \"$Séc\"\n",
            "import a.B as 单位\nprintln \"$单位.X\"\n",
            // an aliased static accessor, used under the alias's property name
            "import static java.lang.Runtime.getRuntime as getRT\nprintln RT.availableProcessors()\n",
            "import static a.Holder.isFlag as isF\nprintln F\n",
            // Hubitat: the library appended in place of `#include` uses the imports
            "import javax.crypto.Cipher\nimport a.B\nimport a.B\nmetadata { }\n#include tuya.tuyaProtocols\n",
            "package p\nimport p.A\n#include a.b\n",
            // the last import of a name wins: these would switch A to q.A
            "package p\nimport q.A\nimport p.A\nprintln A.v\n",
            "package p\nimport q.A as A\nimport p.A\nprintln A.v\n",
            "import p.A\nimport q.A\nimport p.A\nprintln A.v\n",
            "import static p.A.v\nimport static q.A.v\nimport static p.A.v\nprintln v\n",
            "import p.A as X\nimport q.A as X\nimport p.A as X\nprintln X.v\n",
        ] {
            assert_eq!(fixed(src), Fix::Nothing, "{src:?}");
        }
        // still fixed: ASCII names unused in strings, a lone copy, `#include` in a string
        assert_eq!(
            text("import a.B as TU\nprintln \"$T.x\"\n"),
            "println \"$T.x\"\n"
        );
        assert_eq!(text("import p.A\nimport p.A\nA.v\n"), "import p.A\nA.v\n");
        assert_eq!(
            text("package p\nimport p.A\nimport q.B\nA.v\nB.v\n"),
            "package p\nimport q.B\nA.v\nB.v\n"
        );
        assert_eq!(
            text("import a.B\nprintln '#include x'\n"),
            "println '#include x'\n"
        );
    }

    #[test]
    fn removes_same_package_imports_but_not_default_ones() {
        // `import a.C` stays: the file declares a C of its own; `java.util.Date` stays: it
        // may be beating an `a.Date` that this file can't show
        let src = "package a\nimport a.B\nimport a.*\nimport a.C\nimport java.util.Date\nclass C { B b; Date d }\n";
        assert_eq!(
            text(src),
            "package a\nimport a.C\nimport java.util.Date\nclass C { B b; Date d }\n"
        );
        assert_eq!(fixed(&text(src)), Fix::Nothing, "idempotent");
    }

    #[test]
    fn suppressed_violations_are_not_fixed() {
        let src = "// groovyfmt-ignore UnusedImport\nimport a.B\nx = 1; // groovyfmt-ignore\n@SuppressWarnings('UnnecessarySemicolon')\ndef f() { g(); \n}\nimport c.D\n";
        assert_eq!(
            text(src),
            "// groovyfmt-ignore UnusedImport\nimport a.B\nx = 1; // groovyfmt-ignore\n@SuppressWarnings('UnnecessarySemicolon')\ndef f() { g(); \n}\n"
        );
    }

    #[test]
    fn a_line_removed_takes_its_other_reports_along() {
        // `java.util.List` is unused (fixable) and unnecessary (not fixable): removing the
        // line for the first removes the second too, which is not a change to refuse
        assert_eq!(
            text("import java.util.List\nimport a.B\nB.go();\n"),
            "import a.B\nB.go()\n"
        );
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
    fn removes_groovy24_empty_elements() {
        let src = "foo(a,,b)\ndef l = ['off', , 'heat']\ndef m = [a: 1,, b: 2]\nfoo(a, /* x */ , b)\nfoo(a,\n  ,\n  b)\nfoo(a,,b,c) { it }\n";
        assert_eq!(
            text(src),
            "foo(a,b)\ndef l = ['off', 'heat']\ndef m = [a: 1, b: 2]\nfoo(a, /* x */  b)\nfoo(a,\n  b)\nfoo(a,b,c) { it }\n"
        );
        assert_eq!(fixed(&text(src)), Fix::Nothing, "idempotent");
        // Groovy 2.4 rejects a second empty element, or one with a trailing comma, so that
        // code compiled nowhere and is only reported; a trailing comma alone is fine
        for src in [
            "foo(a,,, b)\n",
            "foo(a,,b,,c)\n",
            "def l = [1,,2,]\n",
            "foo(a,,b,)\n",
            "foo(a, b,)\n",
            "def l = [a, b,]\n",
        ] {
            assert_eq!(fixed(src), Fix::Nothing, "{src:?}");
        }
        assert!(lint("foo(a,,b,)\n").unwrap()[0].rule == "Groovy24EmptyElement");
    }

    #[test]
    fn single_quotes_a_string_with_nothing_to_interpolate() {
        let src = r#"def a = "x"
def b = """line
"quoted""""
def c = "tab\t\"q\""
def d = ""
"#;
        assert_eq!(
            text(src),
            r#"def a = 'x'
def b = '''line
"quoted"'''
def c = 'tab\t\"q\"'
def d = ''
"#
        );
        assert_eq!(fixed(&text(src)), Fix::Nothing, "idempotent");
        // a report whose message quotes the strings (ComparisonOfTwoConstants) stays
        assert_eq!(text("x = \"a\" == \"b\"\n"), "x = 'a' == 'b'\n");
        // something to interpolate, a quote, or a `\u` escape (read before the quotes)
        for src in [
            r#"def a = "$x""#,
            r#"def a = "${x}""#,
            r#"def a = "it's""#,
            r#"def a = "\$x""#,
            r#"def a = "'""#,
            "def a = 'x'",
        ] {
            assert_eq!(fixed(src), Fix::Nothing, "{src:?}");
        }
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
        // #211: Groovy 3+ would read `FOO bar()`, a method declaration
        assert_eq!(fixed("FOO;\nbar()\n"), Fix::Nothing);
        assert_eq!(fixed("FOO;;\nbar()\n"), Fix::Nothing);
        // nor may removing an import line join them
        assert_eq!(
            fixed("FOO\nimport a.B\nbar()\n"),
            Fix::Refused("two statements would join into a method declaration on Groovy 3+")
        );
        assert_eq!(
            text("FOO;\nbar();\nfoo;\nbaz()\n"),
            "FOO;\nbar()\nfoo\nbaz()\n"
        );
    }

    #[test]
    fn unparseable_files_are_never_fixed() {
        assert_eq!(
            fixed("import a.B;\ndef x = {\n"),
            Fix::Refused("parse error")
        );
        // a classic `for`'s empty `;` body parses and is the body, so it stays
        assert_eq!(fixed("for (;;);\nf()\n"), Fix::Nothing);
        // the grammar doesn't parse an empty for-in or `if` body, so those aren't either
        for src in ["if (c);\nf()\n", "for (x in xs);\nf()\n"] {
            assert_eq!(fixed(src), Fix::Refused("parse error"), "{src:?}");
        }
    }

    #[test]
    fn only_wanted_rules_are_fixed() {
        let src = "import a.B;\nx = 1;\n";
        let semis = |v: &Violation, _: &str| v.rule == "UnnecessarySemicolon";
        let Fix::Fixed(t, f) = fix(
            Path::new("A.groovy"),
            src,
            None,
            &Context::new(|_| None),
            semis,
        ) else {
            panic!()
        };
        assert_eq!((t.as_str(), f.len()), ("import a.B\nx = 1\n", 2));
        // the import line goes with its `;` even when UnnecessarySemicolon is off
        let imports = |v: &Violation, _: &str| v.rule == "UnusedImport";
        let Fix::Fixed(t, f) = fix(
            Path::new("A.groovy"),
            src,
            None,
            &Context::new(|_| None),
            imports,
        ) else {
            panic!()
        };
        assert_eq!(t, "x = 1;\n");
        assert_eq!(f.len(), 1);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // one range deleted is the common case
    fn the_checks_reject_a_change_of_shape() {
        let del = |range| Edit {
            range,
            with: String::new(),
        };
        // without the empty-body `;` (which lint never flags), `a()` would become the body:
        // same tokens, different tree
        assert_eq!(
            check(
                Path::new("A.groovy"),
                "while (x);\na()\n",
                "while (x)\na()\n",
                &[del(9..10)],
                &[9..9],
                &[],
                &[],
                None,
                &Context::new(|_| None),
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
                &[del(5..6)],
                &[5..5],
                &[],
                &[],
                None,
                &Context::new(|_| None),
                &|_| true
            )
            .is_err()
        );
        assert!(
            check(
                Path::new("A.groovy"),
                "x = 1;\n",
                "x = 1\n",
                &[del(5..6)],
                &[5..5],
                &[],
                &[],
                None,
                &Context::new(|_| None),
                &|_| true
            )
            .is_ok()
        );
        assert_eq!(
            check(
                Path::new("A.groovy"),
                "x = 1;\ny = 2\n",
                "x = 1\ny = 2 +\n",
                &[del(5..6)],
                &[5..5],
                &[],
                &[],
                None,
                &Context::new(|_| None),
                &|_| true
            ),
            Err("the fixed file would not parse")
        );
        // #211: same tokens and tree, but Groovy 3+ reads one method declaration
        assert_eq!(
            check(
                Path::new("A.groovy"),
                "FOO;\nbar()\n",
                "FOO\nbar()\n",
                &[del(3..4)],
                &[3..3],
                &[],
                &[],
                None,
                &Context::new(|_| None),
                &|_| true
            ),
            Err("two statements would join into a method declaration on Groovy 3+")
        );
        // a join already in the input is not the fix's doing
        assert!(
            check(
                Path::new("A.groovy"),
                "FOO\nbar()\nx = 1;\n",
                "FOO\nbar()\nx = 1\n",
                &[del(15..16)],
                &[15..15],
                &[],
                &[],
                None,
                &Context::new(|_| None),
                &|_| true
            )
            .is_ok()
        );
    }

    #[test]
    fn the_checks_reject_a_dependency_that_parses_as_another_call() {
        // what dropping the parentheses would do to these: index a property, call a method on
        // the result, read a property of a list
        for (arg, plain) in [
            ("[0, 1].first()", " [0, 1].first()"),
            ("('x').toUpperCase()", " ('x').toUpperCase()"),
            ("['a:b:1'].size", " ['a:b:1'].size"),
        ] {
            let src = format!("dependencies {{\n    implementation({arg})\n}}\n");
            let out = format!("dependencies {{\n    implementation{plain}\n}}\n");
            let range = 19..src.len() - 3;
            let edit = Edit {
                range: range.clone(),
                with: out[19..out.len() - 3].to_string(),
            };
            let ins = 19..out.len() - 3;
            let run = |calls: &[&str]| {
                check(
                    Path::new("build.gradle"),
                    &src,
                    &out,
                    std::slice::from_ref(&edit),
                    std::slice::from_ref(&ins),
                    calls,
                    &[],
                    None,
                    &Context::new(|_| None),
                    &|_| true,
                )
            };
            assert_eq!(
                run(&["GradleDependencyParentheses"]),
                Err("a rewritten dependency would parse as a different call"),
                "{arg}"
            );
        }
        // the same call, and map notation become a string, pass
        for (src, out) in [
            (
                "dependencies {\n    api(project(':x'))\n}\n",
                "dependencies {\n    api project(':x')\n}\n",
            ),
            (
                "dependencies {\n    api group: 'a', name: 'b'\n}\n",
                "dependencies {\n    api 'a:b'\n}\n",
            ),
        ] {
            let edit = Edit {
                range: 19..src.len() - 3,
                with: out[19..out.len() - 3].to_string(),
            };
            let ins = 19..out.len() - 3;
            assert_eq!(
                check(
                    Path::new("build.gradle"),
                    src,
                    out,
                    &[edit],
                    &[ins],
                    &["GradleDependencyTuple"],
                    &[],
                    None,
                    &Context::new(|_| None),
                    &|_| true
                ),
                Ok(()),
                "{src}"
            );
        }
    }

    #[test]
    fn the_checks_reject_a_space_assignment_that_is_not_the_same_assignment() {
        // `with` for the first line of `src`, from `from` on
        let run = |src: &str, from: &str, with: &str| {
            let range = src.find(from).unwrap()..src.find('\n').unwrap();
            let out = format!("{}{with}{}", &src[..range.start], &src[range.end..]);
            let ins = range.start..range.start + with.len();
            let edit = Edit {
                range,
                with: with.to_string(),
            };
            check(
                Path::new("build.gradle"),
                src,
                &out,
                &[edit],
                &[ins],
                &["GradleSpaceAssignment"],
                &[],
                None,
                &Context::new(|_| None),
                &|_| true,
            )
        };
        let bad = Err("a space assignment would not become the same assignment");
        // another property, another value, still a call, chained to the next part
        assert_eq!(run("version 'a'\n", "v", "group = 'a'"), bad);
        assert_eq!(run("version 'a'\n", "v", "version = 'b'"), bad);
        assert_eq!(run("version 'a'\n", "v", "version('a')"), bad);
        assert_eq!(
            run("group 'a' version 'b'\n", "g", "group = 'a' version = 'b'"),
            bad
        );
        assert_eq!(run("group 'a' version 'b'\n", "v", "version = 'b'"), bad);
        assert_eq!(run("version 'a'\n", "v", "version = 'a'"), Ok(()));
    }

    #[test]
    fn a_fix_failing_the_checks_is_dropped_and_the_others_made() {
        let path = Path::new("build.gradle");
        let src = "apply plugin: 'java'\ndependencies {\n    implementation([0, 1].first())\n    testCompile 'a:b:1'\n}\n";
        let mut todo: Vec<Violation> = lint_file(path, src)
            .unwrap()
            .into_iter()
            .filter(|v| v.fix.is_some())
            .collect();
        assert_eq!(todo.len(), 1, "{todo:?}");
        // a parentheses fix the rule no longer offers for this argument
        let at = src.find("implementation(").unwrap();
        let end = src.find(")\n").unwrap() + 1;
        todo.push(Violation {
            rule: "GradleDependencyParentheses",
            fix: Some(Edit {
                range: at..end,
                with: "implementation [0, 1].first()".into(),
            }),
            span: at..end,
            ..lint_file(path, src).unwrap().remove(0)
        });
        let mut dropped = None;
        let all = |_: &Violation, _: &str| true;
        let Ok(Some((out, done))) = round(
            path,
            src,
            todo,
            &[],
            None,
            &Context::new(|_| None),
            &all,
            &mut dropped,
        ) else {
            panic!()
        };
        assert_eq!(out, src.replace("testCompile", "testImplementation"));
        assert_eq!(done.len(), 1);
        assert_eq!(
            dropped,
            Some("a rewritten dependency would parse as a different call")
        );
    }

    #[test]
    fn failing_fixes_among_many_are_found_in_few_checks() {
        // 400 `;`s to remove, and two edits that break the parse, which fail alone: checking
        // each edit alone took 402 checks of the whole file (#160)
        let path = Path::new("A.groovy");
        let src: String = (0..400).map(|i| format!("x{i} = {i};\n")).collect();
        let mut todo: Vec<Violation> = lint_file(path, &src)
            .unwrap()
            .into_iter()
            .filter(|v| v.fix.is_some())
            .collect();
        assert_eq!(todo.len(), 400);
        for line in [123, 321] {
            let at = src.find(&format!("x{line} = ")).unwrap() + format!("x{line} = ").len();
            let range = at..at + 3;
            todo.push(Violation {
                fix: Some(Edit {
                    range: range.clone(),
                    with: "(".into(),
                }),
                span: range,
                ..lint_file(path, &src).unwrap().remove(0)
            });
        }
        let mut dropped = None;
        let all = |_: &Violation, _: &str| true;
        CHECKS.with(|c| c.set(0));
        let Ok(Some((out, done))) = round(
            path,
            &src,
            todo,
            &[],
            None,
            &Context::new(|_| None),
            &all,
            &mut dropped,
        ) else {
            panic!()
        };
        let checks = CHECKS.with(|c| c.get());
        assert_eq!(out, src.replace(';', ""));
        assert_eq!(done.len(), 400);
        assert_eq!(dropped, Some("the fixed file would not parse"));
        // the whole set, two halves per level down to each bad edit, then the rest: 36
        assert!(checks <= 50, "{checks} checks");
    }

    #[test]
    fn fixes_failing_only_together_refuse_the_file() {
        // a double-quoted mention counts as a use of the import, a single-quoted one doesn't
        let gstrings = |v: &Violation, _: &str| v.rule == "UnnecessaryGString";
        let run = |src: &str| {
            fix(
                Path::new("A.groovy"),
                src,
                None,
                &Context::new(|_| None),
                gstrings,
            )
        };
        // alone, the last mention's fix fails and is dropped, and the other is made
        let Fix::Fixed(t, f) = run("import a.Foo\nx = \"Foo\"\ny = \"z\"\n") else {
            panic!()
        };
        assert_eq!(
            (t.as_str(), f.len()),
            ("import a.Foo\nx = \"Foo\"\ny = 'z'\n", 1)
        );
        // with two, each passes alone but not both: nothing is written
        assert_eq!(
            run("import a.Foo\nx = \"Foo\"\nw = \"Foo\"\ny = \"z\"\n"),
            Fix::Refused("other lint violations would change")
        );
    }
}
