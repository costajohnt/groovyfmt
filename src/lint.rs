//! Native lint rules, named after their CodeNarc equivalents so CI configs map across.
//! Files with parse errors are not linted (a broken tree would produce false positives).

use std::collections::HashSet;
use tree_sitter::{Node, Tree};

#[derive(Debug, PartialEq)]
pub struct Violation {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based
    pub rule: &'static str,
    pub message: String,
}

pub fn lint(src: &str) -> Result<Vec<Violation>, &'static str> {
    let tree = crate::parse(src);
    if tree.root_node().has_error() {
        return Err("parse error");
    }
    let mut v = Vec::new();
    imports(src, &tree, &mut v);
    let mut nodes = Vec::new();
    each(tree.root_node(), &mut |n| nodes.push(n));
    for (i, &n) in nodes.iter().enumerate() {
        match n.kind() {
            "string_literal" => unnecessary_gstring(src, n, &mut v),
            ";" => unnecessary_semicolon(n, nodes.get(i + 1..).unwrap_or(&[]), &mut v),
            "catch_clause" => empty_catch(src, n, &mut v),
            _ => {}
        }
    }
    v.sort_by_key(|x| (x.line, x.col));
    Ok(v)
}

fn each<'a>(n: Node<'a>, f: &mut impl FnMut(Node<'a>)) {
    f(n);
    let mut c = n.walk();
    for child in n.children(&mut c) {
        each(child, f);
    }
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    n.utf8_text(src.as_bytes()).unwrap_or("")
}

fn at(n: Node, rule: &'static str, message: String) -> Violation {
    let p = n.start_position();
    Violation { line: p.row + 1, col: p.column + 1, rule, message }
}

/// UnusedImport + DuplicateImport.
fn imports(src: &str, tree: &Tree, v: &mut Vec<Violation>) {
    fn scan<'a>(n: Node<'a>, src: &'a str, decls: &mut Vec<Node<'a>>, used: &mut HashSet<&'a str>, comments: &mut String) {
        match n.kind() {
            "import_declaration" => decls.push(n), // don't descend: names inside imports aren't uses
            k if k.contains("comment") => {
                comments.push_str(text(n, src));
                comments.push('\n');
            }
            _ if n.child_count() == 0 => {
                used.insert(text(n, src));
            }
            _ => {
                let mut c = n.walk();
                for child in n.children(&mut c) {
                    scan(child, src, decls, used, comments);
                }
            }
        }
    }
    let (mut decls, mut used, mut comments) = (Vec::new(), HashSet::new(), String::new());
    scan(tree.root_node(), src, &mut decls, &mut used, &mut comments);
    let mut seen = HashSet::new();
    for d in decls {
        let full = text(d, src).trim_end_matches(';').split_whitespace().collect::<Vec<_>>().join(" ");
        if !seen.insert(full.clone()) {
            v.push(at(d, "DuplicateImport", format!("Duplicate import: {full}")));
            continue;
        }
        let name = d.child_by_field_name("name").map_or("", |n| text(n, src));
        if name.ends_with('*') || full.ends_with(".*") {
            continue;
        }
        let symbol = d
            .child_by_field_name("alias")
            .map_or_else(|| name.rsplit('.').next().unwrap_or(name), |a| text(a, src));
        // ponytail: a mention in a comment ({@link Foo}) counts as use, as CodeNarc does; no word-boundary check.
        if !symbol.is_empty() && !used.contains(symbol) && !comments.contains(symbol) {
            v.push(at(d, "UnusedImport", format!("The [{name}] import is never referenced")));
        }
    }
}

/// UnnecessaryGString: double-quoted string with nothing to interpolate.
fn unnecessary_gstring(src: &str, n: Node, v: &mut Vec<Violation>) {
    let t = text(n, src);
    let mut c = n.walk();
    let interpolates = n.named_children(&mut c).any(|k| k.kind().starts_with("gstring_"));
    if t.starts_with('"') && !interpolates && !t.contains('\'') && !t.contains('$') {
        v.push(at(n, "UnnecessaryGString", format!("The String {t} can be wrapped in single quotes instead of double quotes")));
    }
}

/// UnnecessarySemicolon: a `;` that ends a line (or the file).
fn unnecessary_semicolon(n: Node, rest: &[Node], v: &mut Vec<Violation>) {
    if n.parent().is_some_and(|p| p.kind().contains("enum") || p.kind() == "for_statement") {
        return;
    }
    let row = n.start_position().row;
    // next leaf in document order, ignoring comments
    let next = rest.iter().find(|x| x.child_count() == 0 && !x.kind().contains("comment"));
    if next.is_none_or(|x| x.start_position().row > row) {
        v.push(at(n, "UnnecessarySemicolon", "Semicolons as line endings can be removed safely".into()));
    }
}

/// EmptyCatchBlock, with CodeNarc's default exemption for `ignore` / `ignored` parameter names.
fn empty_catch(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else { return };
    let param = n.child_by_field_name("parameter").map_or("", |p| text(p, src));
    let ignored = param.split_whitespace().last().is_some_and(|w| w == "ignore" || w == "ignored");
    if body.named_child_count() == 0 && !ignored {
        v.push(at(n, "EmptyCatchBlock", "The catch block is empty".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(src: &str) -> Vec<(&'static str, usize)> {
        lint(src).expect("parses").into_iter().map(|v| (v.rule, v.line)).collect()
    }

    #[test]
    fn imports_unused_duplicate_and_used() {
        let src = "import a.b.Used\nimport a.b.Unused\nimport a.b.Used\nimport a.c.*\nimport a.b.Linked\n\n/** see {@link Linked} */\nclass X { Used u }\n";
        assert_eq!(rules(src), vec![("UnusedImport", 2), ("DuplicateImport", 3)]);
    }

    #[test]
    fn aliased_import_checks_alias() {
        assert_eq!(rules("import a.b.Long as L\nL x = null\n"), vec![]);
        assert_eq!(rules("import a.b.Long as L\nLong x = null\n"), vec![("UnusedImport", 1)]);
    }

    #[test]
    fn gstrings() {
        let src = "def a = \"plain\"\ndef b = \"hi ${a}\"\ndef c = 'single'\ndef d = \"it's\"\n";
        assert_eq!(rules(src), vec![("UnnecessaryGString", 1)]);
    }

    #[test]
    fn semicolons() {
        assert_eq!(rules("def a = 1;\ndef b = 2; def c = 3\n"), vec![("UnnecessarySemicolon", 1)]);
        assert_eq!(rules("for (int i = 0; i < 3; i++) { println i }\n"), vec![]);
    }

    #[test]
    fn empty_catch_blocks() {
        let src = "try { x() } catch (Exception e) { }\ntry { x() } catch (Exception ignored) { }\ntry { x() } catch (Exception e) { log(e) }\n";
        assert_eq!(rules(src), vec![("EmptyCatchBlock", 1)]);
    }

    #[test]
    fn unparseable_is_not_linted() {
        assert_eq!(lint("def x = {\n"), Err("parse error"));
    }
}
