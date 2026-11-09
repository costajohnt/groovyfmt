//! v0 formatter: whitespace-only. Re-indents lines from bracket nesting, strips trailing
//! whitespace, collapses blank-line runs, ensures one final newline. Tokens never change:
//! the output is re-parsed and its token stream compared against the input's before it is
//! accepted, so a wrong heuristic below yields `Skipped`, never a corrupted file.

use tree_sitter::{Node, Tree};

const INDENT: usize = 4; // one step everywhere: corpus measurement (2026-11-08) showed 4 vs 8 for
// brackets/continuations is a wash, and 4 leaves the most files untouched.

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Unchanged,
    Formatted(String),
    Skipped(&'static str),
}

pub fn format(src: &str) -> Outcome {
    let tree = crate::parse(src);
    if tree.root_node().has_error() {
        return Outcome::Skipped("parse error");
    }
    let out = reindent(src, &tree);
    if out == src {
        return Outcome::Unchanged;
    }
    let out_tree = crate::parse(&out);
    if out_tree.root_node().has_error() || tokens(src, &tree) != tokens(&out, &out_tree) {
        return Outcome::Skipped("token mismatch after formatting");
    }
    Outcome::Formatted(out)
}

fn reindent(src: &str, tree: &Tree) -> String {
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
            let indent = indent_for(root, src, pos, &row_indent);
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
            && (node.child_count() == 0 || k.contains("string") || k.contains("comment") || k.contains("doc"))
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
fn indent_for(root: Node, src: &str, pos: usize, row_indent: &[usize]) -> usize {
    let leaf = leaf_at(root, pos);
    let row = leaf.start_position().row;
    let mut child = leaf;
    let mut anchor = leaf.parent();
    let mut after_annotation = false;
    while let Some(a) = anchor {
        after_annotation |= child
            .prev_sibling()
            .is_some_and(|s| s.kind().contains("annotation") || s.kind().contains("modifier"));
        if a.start_position().row < row {
            break;
        }
        child = a;
        anchor = a.parent();
    }
    let Some(a) = anchor.filter(|a| a.parent().is_some()) else { return 0 }; // top level
    let base = row_indent.get(a.start_position().row).copied().unwrap_or(0);

    let first = text(leaf, src);
    let closes_anchor = matches!(first, "}" | ")" | "]") && leaf.parent() == Some(a);
    // Spock blocks (`when:` / `then:`) keep the statement after the label at the label's level.
    if closes_anchor || after_annotation || a.kind() == "labeled_statement" || matches!(first, "else" | "catch" | "finally") {
        return base;
    }
    base + INDENT
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
        match format(s) {
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
        assert_eq!(fmt(src), "def s = '''\n   keep   \n  this\n'''\nprintln s\n");
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
    fn spock_labels_keep_statement_level() {
        let src = "def 'x'() {\nwhen:\ndef a = 1\nthen:\na == 1\n}\n";
        assert_eq!(fmt(src), "def 'x'() {\n    when:\n    def a = 1\n    then:\n    a == 1\n}\n");
    }

    #[test]
    fn skips_unparseable() {
        assert_eq!(format("def x = {\n"), Outcome::Skipped("parse error"));
    }
}
