//! v0 formatter: whitespace-only. Re-indents lines from bracket nesting, strips trailing
//! whitespace, collapses blank-line runs, ensures one final newline. Tokens never change:
//! the output is re-parsed and its token stream compared against the input's before it is
//! accepted, so a wrong heuristic below yields `Skipped`, never a corrupted file.

use crate::text;
use tree_sitter::{Node, Tree, TreeCursor};

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Unchanged,
    Formatted(String),
    Skipped(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// Spaces per level of block nesting (`{ … }`, braceless if/loop bodies, case bodies).
    pub indent: usize,
    /// Spaces for continuation lines: arguments, list/map literals and continued expressions.
    /// `None` means the same as `indent`. IntelliJ's default is 8; across the corpus the two
    /// were equally common, but any one project is consistent.
    pub continuation: Option<usize>,
    /// Indent the statements of a labelled block (Spock `when:` / `then:`) one level deeper
    /// than the label. grails-core uses this style in about half its specs; Spock's docs and
    /// most other code keep them level with the label.
    pub indent_after_labels: bool,
    /// Write leading indentation as tabs: indents are still worked out in columns as above,
    /// with a tab `indent` columns wide, and each full tab width becomes a tab. Any remainder
    /// (a `continuation` that isn't a multiple of `indent`) stays as spaces after the tabs.
    pub use_tabs: bool,
    /// Also normalize the spaces between tokens within a line (see `spacing`).
    pub normalize_spacing: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            indent: 4,
            continuation: None,
            indent_after_labels: false,
            use_tabs: false,
            normalize_spacing: false,
        }
    }
}

impl Style {
    /// Leading whitespace for `cols` columns of indentation.
    fn pad(&self, cols: usize) -> String {
        if self.use_tabs {
            "\t".repeat(cols / self.indent) + &" ".repeat(cols % self.indent)
        } else {
            " ".repeat(cols)
        }
    }

    /// Columns taken by leading whitespace `ws` that is kept as it is. Without tabs a tab
    /// counts as one column, as it always has.
    fn cols(&self, ws: &str) -> usize {
        if self.use_tabs {
            ws.chars()
                .map(|c| if c == '\t' { self.indent } else { 1 })
                .sum()
        } else {
            ws.len()
        }
    }
}

pub fn format(src: &str, style: &Style) -> Outcome {
    format_with(src, style, reindent)
}

/// `format` with the re-indenter as a parameter, so a test can hand the guard a wrong output.
fn format_with(
    src: &str,
    style: &Style,
    reindent: impl Fn(&str, &Tree, &Style) -> Result<String, &'static str>,
) -> Outcome {
    // nothing but whitespace: the file becomes empty, as with rustfmt and prettier
    if src.trim().is_empty() {
        return if src.is_empty() {
            Outcome::Unchanged
        } else {
            Outcome::Formatted(String::new())
        };
    }
    let tree = crate::parse(src);
    if tree.root_node().has_error() {
        return Outcome::Skipped("parse error");
    }
    let out = match reindent(src, &tree, style) {
        Ok(out) => out,
        Err(why) => return Outcome::Skipped(why),
    };
    if out == src {
        return Outcome::Unchanged;
    }
    let out_tree = crate::parse(&out);
    if out_tree.root_node().has_error()
        || tokens(src, &tree) != tokens(&out, &out_tree)
        || (style.normalize_spacing && shape(&tree) != shape(&out_tree))
    {
        if style.normalize_spacing {
            // spacing changed how the code parses: format the file without it
            let plain = Style {
                normalize_spacing: false,
                ..*style
            };
            return format_with(src, &plain, reindent);
        }
        return Outcome::Skipped("token mismatch after formatting");
    }
    Outcome::Formatted(out)
}

/// Every node's kind and number of children, in document order: equal for two trees of
/// the same shape. Spacing can keep the tokens and still change the parse (`a [1]`).
fn shape(tree: &Tree) -> Vec<(u16, usize)> {
    let mut v = Vec::new();
    crate::preorder(tree.root_node(), |n| {
        v.push((n.kind_id(), n.child_count()));
        true
    });
    v
}

/// Indents beyond this many columns mean pathological nesting (generated or hostile input).
/// Giving up keeps output linear in the input instead of growing with depth squared.
const MAX_INDENT: usize = 200;

/// An error when nesting exceeds `MAX_INDENT`, or when spacing doesn't settle (see `spacing`).
fn reindent(src: &str, tree: &Tree, style: &Style) -> Result<String, &'static str> {
    let root = tree.root_node();
    // each output line keeps its own terminator, so mixed line endings (and line breaks
    // inside multi-line strings) survive unchanged
    let mut out: Vec<(String, &str)> = Vec::new();
    // per output line, for `spacing`: its text's source offset and indent length
    let mut bodies: Vec<Option<(usize, usize)>> = Vec::new();
    let mut line_start = 0usize;
    let mut row_indent: Vec<usize> = Vec::new(); // final indent of each row, for children to inherit
    let mut labels = LabelCache::default();
    // asked about each line's start, then its first token, then its end: always forward
    let mut walk = Walk::new(root);
    let segments: Vec<&str> = src.split('\n').collect();
    let last = segments.len() - 1;

    for (i, raw) in segments.iter().enumerate() {
        let line_end = line_start + raw.len(); // offset of the '\n' (or EOF)
        let content = raw.strip_suffix('\r').unwrap_or(raw);
        let eol = if i == last {
            out.last().map_or("\n", |(_, e)| *e) // no terminator in the source: add one
        } else if raw.ends_with('\r') {
            "\r\n"
        } else {
            "\n"
        };
        let lead = content.len() - content.trim_start_matches([' ', '\t']).len();
        let pos = line_start + lead;
        let body = &content[lead..];

        if inside_token(&mut walk, line_start, src) {
            // Line begins inside a multi-line string or comment: emit verbatim.
            row_indent.push(style.cols(&content[..lead]));
            out.push((content.to_string(), eol));
            bodies.push(None);
        } else if body.trim().is_empty() {
            row_indent.push(0);
            if out.last().is_some_and(|(l, _)| !l.is_empty()) {
                out.push((String::new(), eol));
                bodies.push(None);
            }
        } else {
            // `//` at column 0 is commented-out code or a doc-snippet marker; IntelliJ leaves
            // these in place ("comment at first column"), and so do we.
            let indent = if lead == 0 && body.starts_with("//") {
                0
            } else {
                let path = leaf_path(&mut walk, pos);
                indent_for(&path, src, &row_indent, style, &mut labels)
            };
            if indent > MAX_INDENT {
                return Err("nesting too deep to format");
            }
            let keep_trailing = line_end < src.len() && inside_token(&mut walk, line_end, src);
            let text = if keep_trailing { body } else { body.trim_end() };
            row_indent.push(indent);
            let pad = style.pad(indent);
            bodies.push(Some((pos, pad.len())));
            out.push((pad + text, eol));
        }
        line_start = line_end + 1;
    }
    if style.normalize_spacing && !crate::spacing::respace(src, tree, &mut out, &bodies) {
        return Err("spacing did not settle");
    }
    while out.last().is_some_and(|(l, _)| l.is_empty()) {
        out.pop();
    }
    let mut s = String::with_capacity(src.len() + src.len() / 4);
    for (line, eol) in out {
        s.push_str(&line);
        s.push_str(eol);
    }
    Ok(s)
}

/// Spock's block labels. Other labels (`outer:` before a loop) don't open a block.
const SPOCK_LABELS: &[&str] = &[
    "given", "setup", "when", "then", "expect", "and", "where", "cleanup",
];

fn is_spock_label(n: Node, src: &str) -> bool {
    n.kind() == "labeled_statement"
        && n.child_by_field_name("label")
            .is_some_and(|l| SPOCK_LABELS.contains(&text(l, src)))
}

/// Start byte of each block's first Spock label (`None`: no label), computed once per block
/// so the `indent_after_labels` check is a comparison rather than a sibling scan per line.
#[derive(Default)]
struct LabelCache(std::collections::HashMap<usize, Option<usize>>);

impl LabelCache {
    fn first_label(&mut self, block: Node, src: &str) -> Option<usize> {
        *self.0.entry(block.id()).or_insert_with(|| {
            let mut c = block.walk();
            block
                .named_children(&mut c)
                .find(|n| is_spock_label(*n, src))
                .map(|n| n.start_byte())
        })
    }
}

/// A node, and whether its nearest earlier sibling that isn't a comment is an annotation or
/// modifier (a trailing comment after `@Requires(x) // why` is a sibling node too).
type Step<'t> = (Node<'t>, bool);

/// The nodes holding a byte offset, for offsets asked about in increasing order. One cursor
/// moves forward through the tree, so a pass over every line is linear. Asking the tree
/// per line instead (`descendant_for_byte_range`, `parent()`, `prev_sibling()`) restarts
/// from the root or from the first sibling each time: quadratic on long blocks and cubic
/// on a run of indented comments, which are flat siblings.
struct Walk<'t> {
    cursor: TreeCursor<'t>,
    /// The cursor's node and its ancestors, root first.
    stack: Vec<Step<'t>>,
    /// Every child of the cursor's node ends at or before the last offset asked about.
    done: bool,
}

impl<'t> Walk<'t> {
    fn new(root: Node<'t>) -> Self {
        Walk {
            cursor: root.walk(),
            stack: vec![(root, false)],
            done: false,
        }
    }

    /// The nodes whose range holds `byte`, root first and the root always, as
    /// `descendant_for_byte_range(byte, byte + 1)` and its parents would give them (an
    /// error-free tree has no empty nodes). `byte` is never less than the previous one.
    fn seek(&mut self, byte: usize) -> &[Step<'t>] {
        let keep = loop {
            let n = self.cursor.node();
            let depth = self.stack.len();
            if depth > 1 && n.end_byte() <= byte {
                // wholly before `byte`: on to the next sibling, or back up after the last one
                if self.cursor.goto_next_sibling() {
                    let k = n.kind();
                    let after = if k.contains("comment") {
                        self.stack[depth - 1].1
                    } else {
                        k.contains("annotation") || k.contains("modifier")
                    };
                    self.stack[depth - 1] = (self.cursor.node(), after);
                    self.done = false;
                } else {
                    self.cursor.goto_parent();
                    self.stack.pop();
                    self.done = true;
                }
            } else if n.start_byte() > byte {
                // in the gap before this node: its parent is the innermost holder
                break depth.saturating_sub(1).max(1);
            } else if !self.done && self.cursor.goto_first_child() {
                self.stack.push((self.cursor.node(), false));
            } else {
                break depth;
            }
        };
        &self.stack[..keep]
    }
}

/// True when `byte` falls strictly inside a string or comment token (not at its first byte).
fn inside_token(walk: &mut Walk, byte: usize, src: &str) -> bool {
    if byte >= src.len() {
        return false;
    }
    walk.seek(byte).iter().any(|(node, _)| {
        let k = node.kind();
        // ponytail: kind-name sniffing is grammar-specific; the token check in format() catches misses.
        node.start_byte() < byte
            && (node.child_count() == 0
                || k.contains("string")
                || k.contains("comment")
                || k.contains("doc"))
    })
}

/// The leaf at `pos` and its ancestors, root first.
fn leaf_path<'t>(walk: &mut Walk<'t>, pos: usize) -> Vec<Step<'t>> {
    let mut path = walk.seek(pos).to_vec();
    while let Some(c) = path[path.len() - 1]
        .0
        .child(0)
        .filter(|c| c.start_byte() == pos)
    {
        path.push((c, false));
    }
    path
}

/// Indent of the row where the nearest enclosing node that began on an earlier row starts,
/// plus one step, except closers, `else`/`catch`/`finally`, Spock labels, and lines after
/// annotations. `path` is the line's first leaf and its ancestors, root first.
fn indent_for(
    path: &[Step],
    src: &str,
    row_indent: &[usize],
    style: &Style,
    labels: &mut LabelCache,
) -> usize {
    let step = style.indent;
    let leaf = path[path.len() - 1].0;
    let row = leaf.start_position().row;
    // climb while the parent starts on this row; path[i] ends as the anchor's child
    let mut i = path.len() - 1;
    let mut after_annotation = false;
    while i > 0 {
        after_annotation |= path[i].1;
        if path[i - 1].0.start_position().row < row {
            break;
        }
        i -= 1;
    }
    if i < 2 {
        return 0; // top level
    }
    let (child, a, above) = (path[i].0, path[i - 1].0, path[i - 2].0);
    // A `{` body belongs to its declaration or statement, which may have wrapped onto
    // continuation lines before the brace (`void f(String a,\n        String b) {`): indent the
    // body from the line the declaration starts on. Closures keep their own line, since they
    // hang off chained calls.
    let anchor_row = match a.kind() {
        "block" | "class_body" | "enum_body" | "switch_block"
            if !matches!(above.kind(), "block" | "closure" | "source_file") =>
        {
            above.start_position().row
        }
        _ => a.start_position().row,
    };
    let base = row_indent.get(anchor_row).copied().unwrap_or(0);

    let first = text(leaf, src);
    let closes_anchor = matches!(first, "}" | ")" | "]") && path[path.len() - 2].0 == a;
    // the keyword, not a map key or named argument spelled the same (`[else: 1]`)
    let continues = !leaf.is_named() && matches!(first, "else" | "catch" | "finally");
    // A body's `{` on its own line (`void f()` then `{`) lines up with its declaration or
    // statement, as its `}` does.
    let opens_body = first == "{"
        && matches!(
            child.kind(),
            "block" | "class_body" | "enum_body" | "switch_block"
        );
    if closes_anchor || opens_body || after_annotation || continues {
        return base;
    }
    // The statement right after a label (`when:` then a newline) hangs off the label.
    if a.kind() == "labeled_statement" {
        let deeper = style.indent_after_labels && is_spock_label(a, src);
        return base + if deeper { step } else { 0 };
    }
    // Later statements of a Spock block are siblings that follow the block's first label.
    if style.indent_after_labels
        && child.kind() != "labeled_statement"
        && labels
            .first_label(a, src)
            .is_some_and(|start| child.start_byte() > start)
    {
        return base + 2 * step;
    }
    let block_like = a.child(0).is_some_and(|c| text(c, src) == "{")
        || matches!(
            a.kind(),
            "if_statement"
                | "for_statement"
                | "for_in_statement"
                | "while_statement"
                | "do_while_statement"
                | "switch_case"
                | "switch_default"
        );
    base + if block_like {
        step
    } else {
        style.continuation.unwrap_or(step)
    }
}

/// Leaf tokens as (kind, text). Only comments are compared without trailing whitespace,
/// since stripping it from a line comment is a legitimate change; any other leaf, string
/// fragments included, must match exactly.
fn tokens<'a>(src: &'a str, tree: &Tree) -> Vec<(u16, &'a str)> {
    let mut v = Vec::new();
    let mut c = tree.walk();
    'outer: loop {
        let n = c.node();
        if n.child_count() == 0 {
            let t = text(n, src);
            v.push((
                n.kind_id(),
                if n.kind().contains("comment") {
                    t.trim_end()
                } else {
                    t
                },
            ));
        } else if c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                break 'outer;
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(s: &str) -> String {
        match format(s, &Style::default()) {
            Outcome::Unchanged => s.to_string(),
            Outcome::Formatted(o) => o,
            Outcome::Skipped(why) => panic!("skipped: {why}"),
        }
    }

    #[test]
    fn reindents_blocks_and_closures() {
        let src = "class A {\ndef f() {\n  [1, 2].each {\nprintln it\n      }\n}\n}\n";
        let want = "class A {\n    def f() {\n        [1, 2].each {\n            println it\n        }\n    }\n}\n";
        assert_eq!(fmt(src), want);
        assert_eq!(fmt(want), want, "idempotent");
    }

    #[test]
    fn leaves_multiline_strings_alone() {
        let src = "def s = '''\n   keep   \n  this\n'''\nprintln s   \n";
        assert_eq!(
            fmt(src),
            "def s = '''\n   keep   \n  this\n'''\nprintln s\n"
        );
    }

    #[test]
    fn gradle_block_and_blank_lines() {
        let src = "plugins {\n  id 'java'\n}\n\n\n\ndependencies {\n implementation 'a:b:1'\n}";
        let want = "plugins {\n    id 'java'\n}\n\ndependencies {\n    implementation 'a:b:1'\n}\n";
        assert_eq!(fmt(src), want);
    }

    #[test]
    fn nested_openers_on_one_line_indent_once() {
        let src = "foo({\nbar()\n})\n";
        assert_eq!(fmt(src), "foo({\n    bar()\n})\n");
    }

    #[test]
    fn args_and_literals_indent_one_step() {
        let src = "foo(\na,\nb\n)\ndef m = [\nk: 1\n]\n";
        assert_eq!(fmt(src), "foo(\n    a,\n    b\n)\ndef m = [\n    k: 1\n]\n");
    }

    #[test]
    fn closure_inside_case_keeps_case_level() {
        let src = "switch (x) {\ncase 1:\nlist.find { d ->\nprintln d\n}\nbreak\n}\n";
        let want = "switch (x) {\n    case 1:\n        list.find { d ->\n            println d\n        }\n        break\n}\n";
        assert_eq!(fmt(src), want);
    }

    #[test]
    fn chains_annotations_braceless_if_else() {
        let src = "def b = Options.builder()\n.standalone(false)\nclass A {\n@Override\nString toString() {\nif (x)\nreturn 'a'\nelse\nreturn 'b'\n}\n}\n";
        let want = "def b = Options.builder()\n    .standalone(false)\nclass A {\n    @Override\n    String toString() {\n        if (x)\n            return 'a'\n        else\n            return 'b'\n    }\n}\n";
        assert_eq!(fmt(src), want);
    }

    #[test]
    fn body_indents_from_declaration_not_wrapped_line() {
        let src = "class T {\n    void f(String a,\n            String b) {\n        a()\n    }\n    def g() {\n        if (a &&\n                b) {\n            c()\n        } else if (d) {\n            e()\n        }\n        try {\n            f()\n        } catch (IOException |\n                RuntimeException x) {\n            g()\n        }\n    }\n}\n";
        let want = "class T {\n    void f(String a,\n        String b) {\n        a()\n    }\n    def g() {\n        if (a &&\n            b) {\n            c()\n        } else if (d) {\n            e()\n        }\n        try {\n            f()\n        } catch (IOException |\n            RuntimeException x) {\n            g()\n        }\n    }\n}\n";
        assert_eq!(fmt(src), want);
        assert_eq!(fmt(want), want, "idempotent");
    }

    #[test]
    fn brace_on_its_own_line_lines_up_with_its_declaration() {
        let src = "class T\n  {\n  void f()\n      {\n  if (a)\n          {\n  b()\n  }\n  }\n}\n";
        let want = "class T\n{\n    void f()\n    {\n        if (a)\n        {\n            b()\n        }\n    }\n}\n";
        assert_eq!(fmt(src), want);
        assert_eq!(fmt(want), want, "idempotent");
    }

    #[test]
    fn keys_named_like_keywords_keep_their_indent() {
        let src = "def m = [\nelse: 1,\ncatch: 2,\nfinally: 3,\n]\nf(\nelse: 1,\n)\ntry {\na()\n}\ncatch (e) {\n}\nfinally {\n}\n";
        let want = "def m = [\n    else: 1,\n    catch: 2,\n    finally: 3,\n]\nf(\n    else: 1,\n)\ntry {\n    a()\n}\ncatch (e) {\n}\nfinally {\n}\n";
        assert_eq!(fmt(src), want);
    }

    #[test]
    fn first_column_line_comments_stay_put() {
        let src = "def f() {\n// old()\n  // note\n    a()\n}\n";
        assert_eq!(fmt(src), "def f() {\n// old()\n    // note\n    a()\n}\n");
    }

    #[test]
    fn token_guard_sees_whitespace_inside_strings_and_comments() {
        // the safety net that turns a formatter bug into Skipped instead of a changed file
        let toks = |s: &str| {
            let t = crate::parse(s);
            tokens(s, &t)
                .into_iter()
                .map(|(k, x)| (k, x.to_string()))
                .collect::<Vec<_>>()
        };
        assert_ne!(toks("def s = '''a\nb'''\n"), toks("def s = '''a\n  b'''\n"));
        assert_ne!(toks("/* a\n b */\nx()\n"), toks("/* a\n   b */\nx()\n"));
        assert_ne!(
            toks("def s = \"\"\"a\n${x}\"\"\"\n"),
            toks("def s = \"\"\"a\n  ${x}\"\"\"\n")
        );
    }

    #[test]
    fn format_rejects_a_reindent_that_changes_tokens_or_breaks_the_parse() {
        let src = "def s = '''a\n b'''\nif (x) {\nf(1)\n}\n";
        let style = Style::default();
        // the real re-indenter is accepted, so the cases below fail only on the guard
        assert!(matches!(
            format_with(src, &style, reindent),
            Outcome::Formatted(_)
        ));
        let wrong = [
            "def s = '''a\n b'''\nif (x) {\n    f(2)\n}\n", // a token changed
            "def s = '''a\n  b'''\nif (x) {\n    f(1)\n}\n", // a string interior changed
            "def s = '''a\n b'''\nif (x) {\n    f(1)\n}\n}\n", // no longer parses
        ];
        for out in wrong {
            assert_eq!(
                format_with(src, &style, |_, _, _| Ok(out.to_string())),
                Outcome::Skipped("token mismatch after formatting"),
                "{out:?}"
            );
        }
    }

    #[test]
    fn spacing_that_changes_the_tree_falls_back_to_reindenting_only() {
        // `foo (1)` has the tokens of `foo` then `(1)` on the next line, which is two
        // statements: only the tree tells them apart
        let src = "foo\n(1)\n";
        let joined = || Ok("foo (1)\n".to_string());
        assert_eq!(
            format_with(src, &Style::default(), |_, _, _| joined()),
            Outcome::Formatted("foo (1)\n".into()),
            "the token check alone lets it through"
        );
        let spaced = Style {
            normalize_spacing: true,
            ..Style::default()
        };
        let wrong = |s: &str, t: &Tree, style: &Style| {
            if style.normalize_spacing {
                joined()
            } else {
                reindent(s, t, style)
            }
        };
        assert_eq!(format_with(src, &spaced, wrong), Outcome::Unchanged);
    }

    #[test]
    fn mixed_line_endings_are_kept_per_line() {
        let src = "x = 1\r\ndef s = '''a\nb'''\r\nif (y) {\nz()\n}\r\n";
        assert_eq!(
            fmt(src),
            "x = 1\r\ndef s = '''a\nb'''\r\nif (y) {\n    z()\n}\r\n"
        );
        let crlf = "class A {\r\ndef f() {}\r\n}\r\n";
        assert_eq!(fmt(crlf), "class A {\r\n    def f() {}\r\n}\r\n");
    }

    #[test]
    fn trailing_whitespace_inside_multiline_tokens_survives() {
        let src = "def s = '''abc   \nx'''\n/* a   \n b */\n";
        assert_eq!(format(src, &Style::default()), Outcome::Unchanged);
    }

    #[test]
    fn pathological_nesting_is_skipped_not_blown_up() {
        let deep = format!("def x = {}1{}\n", "(\n".repeat(60), "\n)".repeat(60));
        assert_eq!(
            format(&deep, &Style::default()),
            Outcome::Skipped("nesting too deep to format")
        );
    }

    #[test]
    fn non_spock_labels_do_not_open_a_block() {
        let style = Style {
            indent_after_labels: true,
            ..Style::default()
        };
        let src =
            "def f() {\n    outer:\n    for (x in xs) {\n        g()\n    }\n    return 1\n}\n";
        assert_eq!(format(src, &style), Outcome::Unchanged);
    }

    #[test]
    fn continuation_indent_option() {
        let style = Style {
            continuation: Some(8),
            ..Style::default()
        };
        let src = "def f() {\nrender(view: 'x',\nmodel: [a: 1,\nb: 2])\ndef q = Options.builder()\n.x()\nif (a)\ng()\n}\n";
        let want = "def f() {\n    render(view: 'x',\n            model: [a: 1,\n                    b: 2])\n    def q = Options.builder()\n            .x()\n    if (a)\n        g()\n}\n";
        let Outcome::Formatted(out) = format(src, &style) else {
            panic!("not formatted")
        };
        assert_eq!(out, want);
        assert_eq!(format(want, &style), Outcome::Unchanged, "idempotent");
    }

    #[test]
    fn use_tabs_option() {
        let tabs = |continuation| Style {
            use_tabs: true,
            continuation,
            ..Style::default()
        };
        let src = "class A {\n  def f() {\n    render(view: 'x',\n  model: 1)\n    def s = '''\n\t keep\n'''\n  }\n}\n";
        // continuation defaults to one indent: one tab
        let want = "class A {\n\tdef f() {\n\t\trender(view: 'x',\n\t\t\tmodel: 1)\n\t\tdef s = '''\n\t keep\n'''\n\t}\n}\n";
        let Outcome::Formatted(out) = format(src, &tabs(None)) else {
            panic!("not formatted")
        };
        assert_eq!(out, want);
        assert_eq!(format(want, &tabs(None)), Outcome::Unchanged, "idempotent");
        // continuation in columns, a tab being `indent` wide: 8 is two tabs, 6 one and 2 spaces
        let cont = |n, lead: &str| {
            let Outcome::Formatted(out) = format(src, &tabs(Some(n))) else {
                panic!("not formatted")
            };
            assert!(out.contains(&format!("\n{lead}model: 1)")), "{n}: {out:?}");
            assert_eq!(format(&out, &tabs(Some(n))), Outcome::Unchanged, "{n}");
        };
        cont(8, "\t\t\t\t");
        cont(6, "\t\t\t  ");
        // tab-indented input is unchanged with tabs and re-indented with spaces
        assert_eq!(
            fmt(want),
            "class A {\n    def f() {\n        render(view: 'x',\n            model: 1)\n        def s = '''\n\t keep\n'''\n    }\n}\n"
        );
    }

    #[test]
    fn annotation_with_trailing_comment() {
        let src = "class T {\n    @Requires(X) // why\n    def f() {\n        a()\n    }\n}\n";
        assert_eq!(fmt(src), src);
    }

    #[test]
    fn indented_comment_runs_stay_linear() {
        // a sibling walk-back per line took over 120 s on 4000 lines in a release build
        let t = std::time::Instant::now();
        for line in [" // c\n", " /* c */\n"] {
            let body = line.repeat(4000);
            let want = format!(
                "def f() {{\n{}}}\n",
                format!("    {}", line.trim_start()).repeat(4000)
            );
            assert_eq!(fmt(&format!("def f() {{\n{body}}}\n")), want);
            let top = fmt(&body);
            assert_eq!(top, line.trim_start().repeat(4000));
        }
        assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    }

    #[test]
    fn spock_labels_keep_statement_level() {
        let src = "def 'x'() {\nwhen:\ndef a = 1\nthen:\na == 1\n}\n";
        assert_eq!(
            fmt(src),
            "def 'x'() {\n    when:\n    def a = 1\n    then:\n    a == 1\n}\n"
        );
    }

    #[test]
    fn indent_after_labels_option() {
        let style = Style {
            indent_after_labels: true,
            ..Style::default()
        };
        let src = "def 'x'() {\ngiven:\ndef a = 1\ndef b = [\n1\n]\nwhen:\nf(a)\nthen:\na == 1\nwhere:\nx | y\n1 | 2\n}\n";
        let want = "def 'x'() {\n    given:\n        def a = 1\n        def b = [\n            1\n        ]\n    when:\n        f(a)\n    then:\n        a == 1\n    where:\n        x | y\n        1 | 2\n}\n";
        let Outcome::Formatted(out) = format(src, &style) else {
            panic!("not formatted")
        };
        assert_eq!(out, want);
        assert_eq!(format(want, &style), Outcome::Unchanged, "idempotent");
        // a block with no labels is unaffected
        assert_eq!(
            format("def f() {\n    a()\n    b()\n}\n", &style),
            Outcome::Unchanged
        );
    }

    #[test]
    fn skips_unparseable() {
        assert_eq!(
            format("def x = {\n", &Style::default()),
            Outcome::Skipped("parse error")
        );
    }

    #[test]
    fn blank_files_become_empty() {
        let d = Style::default();
        assert_eq!(format("", &d), Outcome::Unchanged);
        assert_eq!(format("  \n\n\t\n", &d), Outcome::Formatted(String::new()));
        assert_eq!(format("\n", &d), Outcome::Formatted(String::new()));
    }

    #[test]
    fn indent_width_is_configurable() {
        assert_eq!(
            format(
                "a {\nb()\n}\n",
                &Style {
                    indent: 2,
                    ..Style::default()
                }
            ),
            Outcome::Formatted("a {\n  b()\n}\n".into())
        );
    }
}
