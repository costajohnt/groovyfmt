pub mod config;
pub mod format;
pub mod lint;

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

pub fn parse(src: &str) -> Tree {
    // one parser per thread, reused: a file is parsed two to four times
    thread_local! {
        static PARSER: std::cell::RefCell<Parser> = std::cell::RefCell::new({
            let mut p = Parser::new();
            p.set_language(&dekobon_tree_sitter_groovy::LANGUAGE.into())
                .expect("grammar ABI mismatch");
            p
        });
    }
    PARSER.with_borrow_mut(|p| p.parse(src, None).expect("parse cancelled"))
}

/// Visit every node in document order. Iterative, so deeply nested input can't overflow the
/// stack; `f` returns false to skip a node's children.
pub fn preorder<'a>(root: Node<'a>, mut f: impl FnMut(Node<'a>) -> bool) {
    let mut c = root.walk();
    loop {
        if f(c.node()) && c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return;
            }
        }
    }
}

/// Source text of a node.
pub fn text<'a>(n: Node, src: &'a str) -> &'a str {
    n.utf8_text(src.as_bytes()).unwrap_or("")
}

/// First ERROR or MISSING node in document order, if any.
pub fn first_error(tree: &Tree) -> Option<Node<'_>> {
    if !tree.root_node().has_error() {
        return None;
    }
    let mut c = tree.walk();
    loop {
        let n = c.node();
        if n.is_error() || n.is_missing() {
            return Some(n);
        }
        if n.has_error() && c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return None;
            }
        }
    }
}

/// Groovy sources, Gradle build scripts, and Jenkinsfiles.
pub fn is_groovy_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    [".groovy", ".gradle", ".gvy", ".gy", ".gsh"]
        .iter()
        .any(|ext| name.ends_with(ext))
        || name == "Jenkinsfile"
}

/// Directories skipped while walking: hidden ones and common build/vendor output.
/// A directory named explicitly on the command line is always entered.
pub fn is_skipped_dir(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.starts_with('.') || matches!(name, "build" | "out" | "target" | "node_modules")
}

#[cfg(test)]
mod tests {
    /// Every node kind and field name matched as a string in format.rs and lint.rs. A grammar
    /// update that renames one would otherwise silently turn a lint rule into a no-op.
    #[test]
    fn grammar_has_every_kind_and_field_we_match() {
        let lang: tree_sitter::Language = dekobon_tree_sitter_groovy::LANGUAGE.into();
        let named = [
            "source_file",
            "identifier",
            "block",
            "class_body",
            "enum_body",
            "closure",
            "switch_block",
            "switch_case",
            "labeled_statement",
            "import_declaration",
            "if_statement",
            "while_statement",
            "for_statement",
            "for_in_statement",
            "try_statement",
            "catch_clause",
            "finally_clause",
            "map_literal",
            "map_entry",
            "binary_expression",
            "assignment_expression",
            "parenthesized_expression",
            "string_literal",
            "number_literal",
            "boolean_literal",
            "null_literal",
            "line_comment",
            "block_comment",
            "groovydoc_comment",
            "annotation",
            "gstring_brace_interpolation",
            "gstring_dollar_interpolation",
        ];
        for k in named {
            assert_ne!(lang.id_for_node_kind(k, true), 0, "missing node kind {k}");
        }
        for k in [";", "=", "==", "{", "}"] {
            assert_ne!(lang.id_for_node_kind(k, false), 0, "missing token {k}");
        }
        let fields = [
            "name",
            "alias",
            "body",
            "parameter",
            "condition",
            "consequence",
            "alternative",
            "key",
            "value",
            "operator",
            "left",
            "right",
            "label",
        ];
        for f in fields {
            assert!(lang.field_id_for_name(f).is_some(), "missing field {f}");
        }
    }
}
