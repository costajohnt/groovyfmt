pub mod config;
pub mod format;
pub mod lint;

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

pub fn parse(src: &str) -> Tree {
    let mut p = Parser::new();
    p.set_language(&dekobon_tree_sitter_groovy::LANGUAGE.into())
        .expect("grammar ABI mismatch");
    p.parse(src, None).expect("parse cancelled")
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
