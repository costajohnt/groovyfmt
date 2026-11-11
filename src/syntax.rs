//! Where a file failed to parse, as messages. They say what groovyfmt could not parse, never
//! that the code is invalid Groovy: the grammar is not the compiler, and about 0.5% of valid
//! files fail to parse.

use crate::text;
use tree_sitter::{Node, Tree};

/// Places reported per file; the rest are only counted.
pub const MAX_REPORTED: usize = 10;

#[derive(Debug, PartialEq)]
pub struct SyntaxError {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based, in characters
    /// Where the reported token ends (where it starts, for a missing one).
    pub end_line: usize,
    pub end_col: usize,
    pub message: String,
}

/// `errors` for source text.
pub fn check(src: &str) -> (Vec<SyntaxError>, usize) {
    errors(src, &crate::parse(src))
}

/// The places in `tree` that did not parse, in document order: the first `MAX_REPORTED`,
/// and how many there are in all. Nothing inside an ERROR node is reported again.
pub fn errors(src: &str, tree: &Tree) -> (Vec<SyntaxError>, usize) {
    let (mut found, mut total) = (Vec::new(), 0);
    crate::preorder(tree.root_node(), |n| {
        if n.is_error() || n.is_missing() {
            total += 1;
            if found.len() < MAX_REPORTED {
                found.push(describe(src, n));
            }
            return false;
        }
        n.has_error()
    });
    (found, total)
}

fn describe(src: &str, n: Node) -> SyntaxError {
    let context = enclosing(n);
    let (at, message) = if n.is_missing() {
        (n, missing(src, n, &context))
    } else {
        unexpected(src, n, &context)
    };
    // tree-sitter columns count bytes; report characters, as lint does
    let pos = |p: tree_sitter::Point| {
        let line = src.split('\n').nth(p.row).unwrap_or("");
        let col = line
            .get(..p.column)
            .map_or(p.column, |prefix| prefix.chars().count());
        (p.row + 1, col + 1)
    };
    let (line, col) = pos(at.start_position());
    // an unparsed run can span many lines; only its first token is pointed at
    let end = if at.is_error() {
        at.start_position()
    } else {
        at.end_position()
    };
    let (end_line, end_col) = pos(end);
    SyntaxError {
        line,
        col,
        end_line,
        end_col,
        message,
    }
}

/// `expected `)`` for a MISSING node, naming the bracket it would close when there is one.
fn missing(src: &str, n: Node, context: &str) -> String {
    let kind = n.kind();
    let what = if n.is_named() {
        words(kind).to_string()
    } else {
        format!("`{kind}`")
    };
    let opener = match kind {
        "}" => Some("{"),
        ")" => Some("("),
        "]" => Some("["),
        _ => None,
    };
    // the nearest earlier sibling opening what this would have closed
    let mut s = n.prev_sibling();
    while let (Some(o), Some(open)) = (s, opener) {
        if o.kind() == open || (open == "{" && o.kind() == "${") {
            let line = o.start_position().row + 1;
            return format!(
                "expected {what} to close the `{}` on line {line}",
                text(o, src)
            );
        }
        s = o.prev_sibling();
    }
    format!("expected {what} {context}")
}

/// For an ERROR node: an unmatched bracket inside it if there is one, since that is usually
/// the cause, else its first token.
fn unexpected<'t>(src: &str, n: Node<'t>, context: &str) -> (Node<'t>, String) {
    let mut leaves = Vec::new();
    crate::preorder(n, |x| {
        if x.child_count() == 0 && x.end_byte() > x.start_byte() {
            leaves.push(x);
        }
        true
    });
    let mut open: Vec<Node> = Vec::new();
    for &l in &leaves {
        let close = match l.kind() {
            "{" | "${" | "(" | "[" | "?[" => {
                open.push(l);
                continue;
            }
            "}" => "{",
            ")" => "(",
            "]" => "[",
            _ => continue,
        };
        match open.pop() {
            Some(o) if o.kind().ends_with(close) => {}
            Some(o) => {
                let line = o.start_position().row + 1;
                return (
                    l,
                    format!(
                        "unexpected `{}`: the `{}` on line {line} is not closed",
                        l.kind(),
                        o.kind()
                    ),
                );
            }
            None => return (l, format!("unexpected `{}` {context}", l.kind())),
        }
    }
    if let Some(o) = open.pop() {
        return (o, format!("`{}` is never closed {context}", o.kind()));
    }
    match leaves.first() {
        Some(&l) => {
            let from = &src[l.start_byte()..n.end_byte().max(l.end_byte())];
            (l, format!("unexpected {} {context}", snippet(from)))
        }
        None if n.end_byte() >= src.len() => (n, format!("unexpected end of file {context}")),
        None => (n, format!("unexpected input {context}")),
    }
}

/// The first line of `t`, at most 30 characters, in backticks, with `…` when cut short.
/// The caller escapes it.
fn snippet(t: &str) -> String {
    let line = t.lines().next().unwrap_or("").trim_end();
    let mut s: String = line.chars().take(30).collect();
    if s.len() < t.trim_end().len() {
        s.push('…');
    }
    format!("`{s}`")
}

/// Where a node sits, in words: its nearest ancestor that is not itself an error.
fn enclosing(n: Node) -> String {
    let mut p = n.parent();
    while let Some(a) = p.filter(|a| a.is_error()) {
        p = a.parent();
    }
    match p.map_or("source_file", |a| a.kind()) {
        "source_file" => "at the top level".into(),
        k => {
            let w = words(k);
            let article = if w.starts_with(['a', 'e', 'i', 'o', 'u']) {
                "an"
            } else {
                "a"
            };
            format!("in {article} {w}")
        }
    }
}

/// The common node kinds in plain words; any other kind with its underscores as spaces.
fn words(kind: &str) -> std::borrow::Cow<'_, str> {
    let w = match kind {
        "identifier" => "name",
        "block" => "block",
        "closure" => "closure",
        "class_body" | "enum_body" | "interface_body" => "class body",
        "class_declaration" => "class declaration",
        "method_declaration" => "method declaration",
        "argument_list" => "argument list",
        "formal_parameters" => "parameter list",
        "formal_parameter" => "parameter",
        "import_declaration" => "import",
        "package_declaration" => "package declaration",
        "local_variable_declaration" | "variable_declarator" => "variable declaration",
        "field_declaration" => "field declaration",
        "binary_expression" | "unary_expression" => "expression",
        "parenthesized_expression" => "parenthesized expression",
        "method_invocation" => "method call",
        "command_chain" => "command call",
        "map_literal" => "map",
        "list_literal" => "list",
        "string_literal" => "string",
        "gstring_brace_interpolation" => "`${}` interpolation",
        "if_statement" => "if statement",
        "for_statement" | "for_in_statement" => "for loop",
        "while_statement" => "while loop",
        "switch_block" => "switch",
        "try_statement" => "try statement",
        "annotation" => "annotation",
        "type_arguments" => "type argument list",
        "subscript_expression" => "subscript",
        "lambda_expression" => "lambda",
        _ => return kind.replace('_', " ").into(),
    };
    w.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(src: &str) -> Vec<String> {
        let (v, total) = errors(src, &crate::parse(src));
        assert_eq!(v.len(), total.min(MAX_REPORTED));
        v.iter()
            .map(|e| format!("{}:{}: {}", e.line, e.col, e.message))
            .collect()
    }

    #[test]
    fn clean_files_have_no_errors() {
        assert_eq!(
            msgs("class A {\n    def f() { 1 }\n}\n"),
            Vec::<String>::new()
        );
        assert_eq!(msgs(""), Vec::<String>::new());
    }

    #[test]
    fn missing_closers_name_their_opener() {
        // tree-sitter puts a missing token right after the last one it read
        assert_eq!(
            msgs("def x = {\n"),
            ["1:10: expected `}` to close the `{` on line 1"]
        );
        assert_eq!(
            msgs("def f(a, b {\n}\n"),
            ["1:11: expected `)` to close the `(` on line 1"]
        );
        assert_eq!(
            msgs("x = [1, 2\ny = 3\n"),
            ["1:10: expected `]` to close the `[` on line 1"]
        );
        assert_eq!(msgs("def s = \"abc\n"), ["1:13: expected `\"` in a string"]);
    }

    #[test]
    fn unbalanced_brackets_inside_an_error_are_the_message() {
        // the `{` of `pipeline` is the one left open
        let src =
            "pipeline {\n  stages {\n    stage(\"x\") {\n      steps { sh \"ls\"\n    }\n  }\n}\n";
        assert_eq!(msgs(src), ["1:10: `{` is never closed at the top level"]);
        let src = "class A {\n  def f() {\n    if (x {\n      y()\n    }\n  }\n}\n";
        assert_eq!(msgs(src), ["3:8: `(` is never closed in a block"]);
        assert_eq!(
            msgs("class A {\n  def f() { return 1 }}\n}\n"),
            ["3:1: unexpected `}` at the top level"]
        );
    }

    #[test]
    fn columns_count_characters() {
        // `é` is two bytes: the `{` ends at byte 23, character 22
        assert_eq!(
            msgs("def s = 'é'; def x = {\n"),
            ["1:23: expected `}` to close the `{` on line 1"]
        );
    }

    #[test]
    fn unparsed_text_is_quoted_from_its_first_token_and_cut_short() {
        assert_eq!(
            msgs("import 'a'\n"),
            ["1:1: unexpected `import` at the top level"]
        );
        assert_eq!(
            msgs("f(a b 'x')\n"),
            ["1:7: unexpected `'x'` in an argument list"]
        );
        assert_eq!(snippet("abc\ndef"), "`abc…`");
        assert_eq!(snippet(&"é".repeat(40)), format!("`{}…`", "é".repeat(30)));
        assert_eq!(snippet("ab  \n"), "`ab`");
        assert_eq!(
            msgs("def f() { x = }\n"),
            ["1:13: unexpected `=` in a block"]
        );
    }

    #[test]
    fn at_most_ten_are_reported_and_all_are_counted() {
        let src: String = (0..25).map(|i| format!("def f{i}() {{ x = }}\n")).collect();
        let (v, total) = errors(&src, &crate::parse(&src));
        assert_eq!((v.len(), total), (MAX_REPORTED, 25));
        assert_eq!(v[9].line, 10);
    }
}
