//! Where a file failed to parse, as messages. They say what groovyfmt could not parse, never
//! that the code is invalid Groovy: the grammar is not the compiler, and about 0.5% of valid
//! files fail to parse. A few constructs that parse are also reported because Groovy 2.4, 3
//! and 4 all reject them (`rejected`).

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

/// `errors` for source text, and the places that parse but that Groovy 2.4, 3 and 4 all
/// reject (`rejected`).
pub fn check(src: &str) -> (Vec<SyntaxError>, usize) {
    let tree = crate::parse(src);
    let (mut found, mut total) = errors(src, &tree);
    let extra = rejected(src, &tree);
    total += extra.len();
    found.extend(extra);
    found.sort_by_key(|e| (e.line, e.col));
    found.truncate(MAX_REPORTED);
    (found, total)
}

/// What the grammar takes leniently and Groovy 2.4, 3 and 4 reject: a shebang after the first
/// line, as after a license comment (only the top level holds one); a second `package`
/// statement; a `$` in a double-quoted string that starts no name or `${`, as in `$(` or
/// `$[`; a backslash in a quoted string that starts no escape, `'a\d+'`; a label on a
/// `return`'s line, which is a command call as its value, `return readJSON text: x`. The
/// last three are found from the text, so a file without them costs no walk of the tree.
// ponytail: a primitive type name as a variable's name (`{ char -> }`) is rejected by Groovy
// too, but where the grammar misreads a valid declaration the name lands on `int` or `boolean`,
// so checking it reported valid files.
fn rejected(src: &str, tree: &Tree) -> Vec<SyntaxError> {
    let root = tree.root_node();
    let mut out = Vec::new();
    let mut c = root.walk();
    let mut packages = 0;
    for n in root.children(&mut c) {
        if n.kind() == "shebang" && n.start_position().row > 0 {
            out.push(at(
                src,
                n,
                "a shebang (`#!`) must be on the file's first line",
            ));
        }
        if n.kind() == "package_declaration" {
            packages += 1;
            if packages > 1 {
                out.push(at(src, n, "a file has at most one `package` statement"));
            }
        }
    }
    // Not after an unclosed `${` in the string's text: where the grammar takes a `${` for text
    // (`"""{"a": "${f('$.x')}"}"""`, after a `"` in a triple-quoted string) its code is text too.
    let in_gstring = |i: usize| {
        root.descendant_for_byte_range(i, i + 1)
            .filter(|n| n.kind() == "string_fragment")
            .and_then(|n| n.parent())
            .is_some_and(|s| {
                let before = &src[s.start_byte()..i];
                s.kind() == "string_literal"
                    && before.starts_with('"')
                    && before.rfind("${").is_none_or(|o| before[o..].contains('}'))
            })
    };
    let mut push = |i: usize, len: usize, message: String| {
        let (line, col) = position(src, byte_point(src, i));
        out.push(SyntaxError {
            line,
            col,
            end_line: line,
            end_col: col + len,
            message,
        });
    };
    // A `$` in a double-quoted string that starts no name or `${`: `"$(x)"`, `"$[x]"`, `"a$"`.
    for (i, _) in src.match_indices('$') {
        let escaped = src[..i].bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1;
        let next = src[i + 1..].chars().next();
        if escaped
            || next.is_some_and(|c| c == '{' || c == '_' || c.is_alphabetic())
            || !in_gstring(i)
        {
            continue;
        }
        let message = if next == Some('(') {
            "`$(` in a double-quoted string: write `${...}`, or `\\$` for a dollar sign".into()
        } else {
            "`$` in a double-quoted string must start a name or `${...}`: write `\\$` for a dollar sign"
                .into()
        };
        push(i, 1 + usize::from(next == Some('(')), message);
    }
    // A backslash in a quoted string that starts no escape: `'a\d+'` (a slashy string,
    // `/a\d+/`, takes it).
    let mut after_escape = 0;
    for (i, _) in src.match_indices('\\') {
        if i < after_escape {
            continue;
        }
        let rest = &src[i + 1..];
        let Some(c) = rest.chars().next() else { break };
        after_escape = i + 1 + c.len_utf8();
        let valid = match c {
            'b' | 't' | 'n' | 'f' | 'r' | 's' | '"' | '\'' | '\\' | '$' | '\n' | '\r' => true,
            '0'..='7' => true,
            'u' => {
                let hex = rest.trim_start_matches('u');
                hex.len() >= 4 && hex.as_bytes()[..4].iter().all(u8::is_ascii_hexdigit)
            }
            _ => false,
        };
        if valid {
            continue;
        }
        let quoted = root
            .descendant_for_byte_range(i, i + 1)
            .is_some_and(|n| n.kind() == "string_literal" && text(n, src).starts_with('\''))
            || in_gstring(i);
        if quoted {
            push(
                i,
                2,
                format!(
                    "`\\{c}` is not an escape in a quoted string: write `\\\\{c}`, or use a slashy string"
                ),
            );
        }
    }
    for (i, _) in src.match_indices("return") {
        let Some(r) = root
            .descendant_for_byte_range(i, i + 6)
            .filter(|n| n.kind() == "return")
            .and_then(|n| n.parent())
            .filter(|r| r.kind() == "return_statement")
        else {
            continue;
        };
        // Only a label: a name argument (`return foo x`) is rejected too, but where the grammar
        // misreads a valid value (`return pipeline { }`, `return y.new X()`) a statement
        // follows the same way.
        if let Some(next) = r.next_sibling().filter(|n| {
            n.kind() == "labeled_statement"
                && n.start_position().row == r.end_position().row
                && !src[r.start_byte()..n.start_byte()]
                    .trim_end()
                    .ends_with(';')
        }) {
            out.push(at(
                src,
                next,
                "a `return` value can't be a command call: put its arguments in parentheses",
            ));
        }
    }
    out
}

fn at(src: &str, n: Node, message: &str) -> SyntaxError {
    let (line, col) = position(src, n.start_position());
    let (end_line, end_col) = position(src, n.end_position());
    SyntaxError {
        line,
        col,
        end_line,
        end_col,
        message: message.into(),
    }
}

/// The tree-sitter point of byte `i`.
fn byte_point(src: &str, i: usize) -> tree_sitter::Point {
    let before = &src[..i];
    let row = before.matches('\n').count();
    let column = i - before.rfind('\n').map_or(0, |n| n + 1);
    tree_sitter::Point { row, column }
}

/// A tree-sitter point as a 1-based line and column; tree-sitter columns count bytes, these
/// count characters, as lint does.
fn position(src: &str, p: tree_sitter::Point) -> (usize, usize) {
    let line = src.split('\n').nth(p.row).unwrap_or("");
    let col = line
        .get(..p.column)
        .map_or(p.column, |prefix| prefix.chars().count());
    (p.row + 1, col + 1)
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
    let pos = |p| position(src, p);
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

    fn checked(src: &str) -> Vec<String> {
        check(src)
            .0
            .iter()
            .map(|e| format!("{}:{}: {}", e.line, e.col, e.message))
            .collect()
    }

    #[test]
    fn constructs_groovy_rejects_are_reported() {
        let shebang = "a shebang (`#!`) must be on the file's first line";
        assert_eq!(
            checked("// license\n#!/usr/bin/env groovy\nprintln 1\n"),
            [format!("2:1: {shebang}")]
        );
        assert_eq!(
            checked("#!/usr/bin/env groovy\nprintln 1\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            checked("package a\npackage b\n"),
            ["2:1: a file has at most one `package` statement"]
        );
        let dollar = "`$(` in a double-quoted string: write `${...}`, or `\\$` for a dollar sign";
        assert_eq!(
            checked("x = \"a $(uname) b\"\ny = \"\"\"\n$(z)\"\"\"\n"),
            [format!("1:8: {dollar}"), format!("3:1: {dollar}")]
        );
        // escaped, single-quoted, slashy, dollar-slashy, in a GString's code: Groovy takes them
        assert_eq!(
            checked("a = \"\\$(x)\"\nb = '$(x)'\nc = /$(x)/\nd = $/ $(x) /$\ne = \"${f('$(')}\"\n"),
            Vec::<String>::new()
        );
        let lone = "`$` in a double-quoted string must start a name or `${...}`: write `\\$` for a dollar sign";
        assert_eq!(
            checked("a = \"$[x]\"\nb = \"\"\"cost: $5\"\"\"\nc = \"a$\"\n"),
            [
                format!("1:6: {lone}"),
                format!("2:14: {lone}"),
                format!("3:7: {lone}")
            ]
        );
        assert_eq!(
            checked("a = \"$x ${y} \\$5 $\u{e9}\"\nb = '$5'\nc = /a$/\nd = $/ $5 /$\n"),
            Vec::<String>::new()
        );
        // code in a `${}` the grammar takes for text, after a `"` in a triple-quoted string
        assert_eq!(
            checked("a = \"\"\"{\"s\": \"${f('$.x', '\\d')}\"}\"\"\"\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            checked("a = 'a\\d+'\nb = '''\\e'''\nc = \"x\\.y\"\nd = \"${'\\w'}\"\n"),
            [
                "1:7: `\\d` is not an escape in a quoted string: write `\\\\d`, or use a slashy string",
                "2:8: `\\e` is not an escape in a quoted string: write `\\\\e`, or use a slashy string",
                "3:7: `\\.` is not an escape in a quoted string: write `\\\\.`, or use a slashy string",
                "4:9: `\\w` is not an escape in a quoted string: write `\\\\w`, or use a slashy string",
            ]
        );
        // escapes Groovy takes (`\s` in 3 and 4), a slashy string's backslashes, comments
        assert_eq!(
            checked(
                "a = '\\b\\t\\n\\f\\r\\s\\\"\\'\\\\\\$\\0\\17\\u00e9 \\\\d'\nb = \"\\\\d \\$ \\u0041\"\nc = /a\\d/\nd = $/\\d/$\ne = 'a\\\nb' // \\d\n"
            ),
            Vec::<String>::new()
        );
        let ret = "a `return` value can't be a command call: put its arguments in parentheses";
        assert_eq!(
            checked("def f(x) {\n    return readJSON text: x\n}\ndef g(x) { return foo a: x }\n"),
            [format!("2:21: {ret}"), format!("4:23: {ret}")]
        );
        assert_eq!(
            checked(
                "def f(x) {\n    if (x) return 1 else return 2\n    return x; a: y\n    return pipeline { }\n    return y.new X()\n}\nreturnValue x\n"
            ),
            Vec::<String>::new()
        );
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
