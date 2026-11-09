pub mod format;
pub mod lint;

use tree_sitter::{Language, Node, Parser, Tree};

pub fn language() -> Language {
    #[cfg(feature = "g-dekobon")]
    return dekobon_tree_sitter_groovy::LANGUAGE.into();
    #[cfg(feature = "g-amaanq")]
    return ts_groovy_amaanq::LANGUAGE.into();
    #[cfg(feature = "g-murtaza")]
    return ts_groovy_murtaza::language();
}

pub fn parse(src: &str) -> Tree {
    let mut p = Parser::new();
    p.set_language(&language()).expect("grammar ABI mismatch");
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

pub fn is_groovy_file(path: &std::path::Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".groovy") || name.ends_with(".gradle") || name.ends_with(".gvy") || name == "Jenkinsfile"
}
