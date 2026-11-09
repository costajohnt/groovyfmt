//! v0 formatter: whitespace-only. Re-indents lines from bracket nesting, strips trailing
//! whitespace, collapses blank-line runs, ensures one final newline. Tokens never change:
//! the output is re-parsed and its token stream compared against the input's before it is
//! accepted, so a wrong heuristic below yields `Skipped`, never a corrupted file.

use tree_sitter::{Node, Tree};

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Unchanged,
    Formatted(String),
    Skipped(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// Spaces per level. The same step is used for blocks, brackets and continuation lines:
    /// on a 23.8k-file corpus, separate continuation widths made no difference.
    pub indent: usize,
    /// Indent the statements of a labelled block (Spock `when:` / `then:`) one level deeper
    /// than the label. grails-core uses this style in about half its specs; Spock's docs and
    /// most other code keep them level with the label.
    pub indent_after_labels: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            indent: 4,
            indent_after_labels: false,
        }
    }
}

pub fn format(src: &str, style: &Style) -> Outcome {
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
    let out = reindent(src, &tree, style);
    if out == src {
        return Outcome::Unchanged;
    }
    let out_tree = crate::parse(&out);
    if out_tree.root_node().has_error() || tokens(src, &tree) != tokens(&out, &out_tree) {
        return Outcome::Skipped("token mismatch after formatting");
    }
    Outcome::Formatted(out)
}

fn reindent(src: &str, tree: &Tree, style: &Style) -> String {
    let eol = if src.contains("\r\n") { "\r\n" } else { "\n" };
    let root = tree.root_node();
    let mut out: Vec<String> = Vec::new();
    let mut line_start = 0usize;
    let mut row_indent: Vec<usize> = Vec::new(); // final indent of each row, for children to inherit

    for raw in src.split('\n') {
        let line_end = line_start + raw.len(); // offset of the '\n' (or EOF)
        let content = raw.strip_suffix('\r').unwrap_or(raw);
        let lead = content.len() - content.trim_start_matches([' ', '\t']).len();
        let pos = line_start + lead;
        let body = &content[lead..];

        if inside_token(root, line_start, src) {
            // Line begins inside a multi-line string or comment: emit verbatim.
            row_indent.push(lead);
            out.push(content.to_string());
        } else if body.trim().is_empty() {
            row_indent.push(0);
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
        } else {
            let keep_trailing = line_end < src.len() && inside_token(root, line_end, src);
            let text = if keep_trailing { body } else { body.trim_end() };
            // `//` at column 0 is commented-out code or a doc-snippet marker; IntelliJ leaves
            // these in place ("comment at first column"), and so do we.
            let indent = if lead == 0 && body.starts_with("//") {
                0
            } else {
                indent_for(root, src, pos, &row_indent, style)
            };
            row_indent.push(indent);
            out.push(format!("{}{}", " ".repeat(indent), text));
        }
        line_start = line_end + 1;
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let mut s = out.join(eol);
    s.push_str(eol);
    s
}

/// True when `byte` falls strictly inside a string or comment token (not at its first byte).
fn inside_token(root: Node, byte: usize, src: &str) -> bool {
    if byte >= src.len() {
        return false;
    }
    let mut n = root.descendant_for_byte_range(byte, byte + 1);
    while let Some(node) = n {
        let k = node.kind();
        // ponytail: kind-name sniffing is grammar-specific; the token check in format() catches misses.
        if node.start_byte() < byte
            && (node.child_count() == 0
                || k.contains("string")
                || k.contains("comment")
                || k.contains("doc"))
        {
            return true;
        }
        n = node.parent();
    }
    false
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    n.utf8_text(src.as_bytes()).unwrap_or("")
}

fn leaf_at(root: Node, pos: usize) -> Node {
    let mut n = root.descendant_for_byte_range(pos, pos).unwrap_or(root);
    while n.child_count() > 0 {
        match n.child(0) {
            Some(c) if c.start_byte() == pos => n = c,
            _ => break,
        }
    }
    n
}

/// Indent of the row where the nearest enclosing node that began on an earlier row starts,
/// plus one step, except closers, `else`/`catch`/`finally`, Spock labels, and lines after annotations.
fn indent_for(root: Node, src: &str, pos: usize, row_indent: &[usize], style: &Style) -> usize {
    let step = style.indent;
    let leaf = leaf_at(root, pos);
    let row = leaf.start_position().row;
    let mut child = leaf;
    let mut anchor = leaf.parent();
    let mut after_annotation = false;
    while let Some(a) = anchor {
        // a trailing comment after the annotation (`@Requires(x) // why`) is a sibling node too
        after_annotation |= std::iter::successors(child.prev_sibling(), |s| s.prev_sibling())
            .find(|s| !s.kind().contains("comment"))
            .is_some_and(|s| s.kind().contains("annotation") || s.kind().contains("modifier"));
        if a.start_position().row < row {
            break;
        }
        child = a;
        anchor = a.parent();
    }
    let Some(a) = anchor.filter(|a| a.parent().is_some()) else {
        return 0;
    }; // top level
    // A `{` body belongs to its declaration or statement, which may have wrapped onto
    // continuation lines before the brace (`void f(String a,\n        String b) {`): indent the
    // body from the line the declaration starts on. Closures keep their own line, since they
    // hang off chained calls.
    let anchor_row = match a.kind() {
        "block" | "class_body" | "enum_body" | "switch_block" => {
            a.parent()
                .filter(|p| !matches!(p.kind(), "block" | "closure" | "source_file"))
                .unwrap_or(a)
                .start_position()
                .row
        }
        _ => a.start_position().row,
    };
    let base = row_indent.get(anchor_row).copied().unwrap_or(0);

    let first = text(leaf, src);
    let closes_anchor = matches!(first, "}" | ")" | "]") && leaf.parent() == Some(a);
    if closes_anchor || after_annotation || matches!(first, "else" | "catch" | "finally") {
        return base;
    }
    // The statement right after a label (`when:` then a newline) hangs off the label.
    if a.kind() == "labeled_statement" {
        return base + if style.indent_after_labels { step } else { 0 };
    }
    // Later statements of a labelled block are siblings of the labeled_statement.
    // ponytail: scans earlier siblings per line, quadratic in block length; fine for test methods.
    if style.indent_after_labels
        && child.kind() != "labeled_statement"
        && std::iter::successors(child.prev_named_sibling(), |s| s.prev_named_sibling())
            .any(|s| s.kind() == "labeled_statement")
    {
        return base + 2 * step;
    }
    base + step
}

/// Leaf tokens as (kind, text), trailing whitespace trimmed (line comments may lose it).
fn tokens<'a>(src: &'a str, tree: &Tree) -> Vec<(u16, &'a str)> {
    let mut v = Vec::new();
    let mut c = tree.walk();
    'outer: loop {
        let n = c.node();
        if n.child_count() == 0 {
            v.push((n.kind_id(), text(n, src).trim_end()));
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
    fn first_column_line_comments_stay_put() {
        let src = "def f() {\n// old()\n  // note\n    a()\n}\n";
        assert_eq!(fmt(src), "def f() {\n// old()\n    // note\n    a()\n}\n");
    }

    #[test]
    fn annotation_with_trailing_comment() {
        let src = "class T {\n    @Requires(X) // why\n    def f() {\n        a()\n    }\n}\n";
        assert_eq!(fmt(src), src);
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
