//! `normalize_spacing`: the spaces between tokens on a line, decided from the parse tree.
//! Runs after re-indenting, on its output lines, and only ever changes a run of spaces
//! between two tokens on the same line (never a newline, never inside a string or comment).
//!
//! A run of two or more spaces (or a tab) may be alignment, so it changes only when the token
//! after it lines up with nothing on a nearby line (see `neighbors` and `token_at`). Gaps next
//! to a comment and everything from a Spock `where:` label to the end of its block are left
//! alone. When a change earlier on a line moves the tokens after it, the next run that is kept
//! absorbs the shift, and a line whose changes would still move an aligned `=`, `:` or comment
//! out of line keeps its spacing up to it (see `respace`).
//!
//! `format` re-parses the result and requires the same tokens and the same tree shape, so a
//! rule that changed how the code parses would be rejected and the file formatted without
//! spacing changes; the rules below leave the ambiguous shapes (`a -b`, `foo (x).y`) alone so
//! the check doesn't have to.

use std::cell::Cell;
use tree_sitter::{Node, Tree};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Want {
    /// leave the gap as it is
    Keep,
    /// no space
    Zero,
    /// one space
    One,
    /// a run of spaces becomes one; a single space or none stays
    Collapse,
}

/// The whitespace between two tokens on one line, and what it should be.
#[derive(Debug)]
struct Gap {
    start: usize,
    end: usize,
    want: Want,
    rule: &'static str,
}

/// A leaf token: a node without children, or a whole string or comment.
struct Tok<'t> {
    node: Node<'t>,
    parent: &'static str,
    field: Option<&'static str>,
    /// in a `where:` block
    table: bool,
    /// a brace of a closure that stands alone as a statement or follows `$`
    odd: bool,
}

pub(crate) fn is_atom(n: Node) -> bool {
    let k = n.kind();
    n.child_count() == 0
        || k.contains("string")
        || k.contains("comment")
        || k == "quoted_identifier"
}

/// A statement at or after the `where:` label of its block, where Spock's data tables
/// (`a | b || c`) and data pipes live.
fn is_table(n: Node, src: &str, after_where: &mut bool) -> bool {
    if n.kind() == "labeled_statement"
        && n.child_by_field_name("label")
            .is_some_and(|l| crate::text(l, src) == "where")
    {
        *after_where = true;
    }
    *after_where
}

/// An ancestor on the walk: the node, its field in its parent, whether it is a table row,
/// and whether a `where:` label came among its children so far.
type Anc<'t> = (Node<'t>, Option<&'static str>, bool, bool);

/// Every gap between two tokens on the same line, in source order.
fn gaps(src: &str, tree: &Tree) -> Vec<Gap> {
    let mut out = Vec::new();
    let mut c = tree.walk();
    let mut anc: Vec<Anc> = Vec::new();
    let mut tables = 0usize;
    let mut prev: Option<Tok> = None;
    loop {
        let n = c.node();
        if is_atom(n) {
            let tok = Tok {
                node: n,
                parent: anc.last().map_or("", |a| a.0.kind()),
                field: c.field_name(),
                table: tables > 0 || anc.last().is_some_and(|a| a.3),
                odd: match anc.as_slice() {
                    [.., up, (p, ..)] if p.kind() == "closure" => {
                        up.0.kind() == "expression_statement"
                            || src[..p.start_byte()].ends_with('$')
                    }
                    _ => false,
                },
            };
            if let Some(a) = &prev {
                let (s, e) = (a.node.end_byte(), n.start_byte());
                if a.node.end_position().row == n.start_position().row
                    && src.as_bytes()[s..e]
                        .iter()
                        .all(|b| matches!(b, b' ' | b'\t'))
                {
                    let (want, rule) = want(a, &tok, &anc, src);
                    out.push(Gap {
                        start: s,
                        end: e,
                        want,
                        rule,
                    });
                }
            }
            prev = Some(tok);
        } else {
            let mut after_where = anc.last().is_some_and(|a| a.3);
            let t = is_table(n, src, &mut after_where);
            if let Some(a) = anc.last_mut() {
                a.3 = after_where;
            }
            tables += usize::from(t);
            anc.push((n, c.field_name(), t, false));
            c.goto_first_child();
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return out;
            }
            if anc.pop().is_some_and(|a| a.2) {
                tables -= 1;
            }
        }
    }
}

/// Nodes whose `{` … `}` get one space inside and one before.
const BODIES: &[&str] = &[
    "block",
    "closure",
    "class_body",
    "enum_body",
    "switch_block",
];

/// Parents whose anonymous children (other than brackets and commas) are infix operators.
const INFIX: &[&str] = &[
    "binary_expression",
    "assignment_expression",
    "spaceship_expression",
    "regex_find_expression",
    "regex_match_expression",
    "power_expression",
    "identity_expression",
    "logical_implication_expression",
    "elvis_expression",
    "ternary_expression",
    "membership_expression",
    "instanceof_expression",
    "cast_expression",
    "lambda_expression",
];

/// Parents where `=` gives a declaration or parameter its value. Not a closure parameter's
/// default (`{ int b=2 -> }`): the grammar reads `b = 2 ->` differently, so it stays as written.
const DECLARES: &[&str] = &[
    "variable_declarator",
    "local_variable_declaration",
    "field_declaration",
    "multi_assignment_declaration",
    "multi_assignment_expression",
    "formal_parameter",
];

const UNARY: &[&str] = &[
    "unary_expression",
    "update_expression",
    "unary_update_expression",
    "spread_arguments",
];

/// Member access and ranges, whose spacing is left as written.
const MEMBER: &[&str] = &[
    ".", "?.", "*.", ".&", ".@", "*.@", "?.@", "??.", ".*", "..", "..<", "<..", "<..<", "...", "@",
];

/// Where a call's `(` may lose the space before it: the call is a whole statement, argument,
/// value or element. Anywhere else (`foo (x).y`, `foo (x) + 1`) the space may be what makes
/// Groovy read it as a command call with a parenthesized argument, so it stays.
const CALL_CONTEXTS: &[&str] = &[
    "expression_statement",
    "argument_list",
    "variable_declarator",
    "local_variable_declaration",
    "return_statement",
    "list_literal",
    "map_entry",
    "named_argument",
];

fn is_op(t: &Tok) -> bool {
    let k = t.node.kind();
    !t.node.is_named()
        && ((INFIX.contains(&t.parent) && !matches!(k, "(" | ")" | "[" | "]" | ","))
            || (k == "=" && DECLARES.contains(&t.parent))
            || (k == "->" && matches!(t.parent, "closure_parameters" | "switch_arrow_case"))
            || (matches!(k, "in" | ":") && t.parent == "for_in_statement"))
}

/// `a -b`: a space before a binary operator and none after reads as a prefix operator on a
/// command argument, whatever the parser made of it; left as written.
pub(crate) fn lopsided(op: Node, src: &str) -> bool {
    let b = src.as_bytes();
    let before = op.start_byte() > 0 && matches!(b[op.start_byte() - 1], b' ' | b'\t');
    let after = b
        .get(op.end_byte())
        .is_some_and(|c| c.is_ascii_whitespace());
    before && !after
}

/// `x as Kind<Path>[]` read as `Kind < Path > []`: a `<` or `>` comparison chained with the
/// other one is a generic type the grammar didn't recognize; spacing it would change the parse.
pub(crate) fn generic_like(op: Node) -> bool {
    let cmp = |n: Option<Node>| {
        n.is_some_and(|n| {
            n.kind() == "binary_expression"
                && n.child_by_field_name("operator")
                    .is_some_and(|o| matches!(o.kind(), "<" | ">"))
        })
    };
    op.parent().is_some_and(|e| {
        cmp(e.parent()) || cmp(e.child_by_field_name("left")) || cmp(e.child_by_field_name("right"))
    })
}

/// The spacing between `a` and the next token `b` on its line. `anc` holds `b`'s ancestors.
fn want(a: &Tok, b: &Tok, anc: &[Anc], src: &str) -> (Want, &'static str) {
    use Want::*;
    let (ak, bk) = (a.node.kind(), b.node.kind());
    let (pa, pb) = (a.parent, b.parent);
    if a.table || b.table || ak.contains("comment") || bk.contains("comment") {
        return (Keep, "");
    }
    if MEMBER.contains(&ak) || MEMBER.contains(&bk) {
        return (Keep, "");
    }
    // commas and semicolons
    if bk == "," {
        return if matches!(ak, "(" | "[" | ",") {
            (Keep, "")
        } else {
            (Zero, "comma")
        };
    }
    if ak == "," {
        return if matches!(bk, ")" | "]" | "}") {
            (Keep, "")
        } else {
            (One, "comma")
        };
    }
    if bk == ";" {
        // `while (x) ;`: the `;` is the loop body
        return if b.field.is_some() || matches!(ak, "(" | ";") {
            (Keep, "")
        } else {
            (Zero, "semicolon")
        };
    }
    if ak == ";" {
        return if matches!(bk, ")" | ";") {
            (Keep, "")
        } else {
            (One, "semicolon")
        };
    }
    // inside parentheses and brackets
    if ak == "(" || bk == ")" {
        return (Zero, "parens");
    }
    if ak == "[" || bk == "]" {
        return (Zero, "brackets");
    }
    // `String []`, `new int [5]`; a subscript `a [1]` stays as written
    if bk == "[" && matches!(pb, "array_type" | "dimension" | "dimension_expression") {
        return (Zero, "brackets");
    }
    // generics: `List<String>`, `class A<T>`
    let generic = |p: &str| matches!(p, "type_arguments" | "type_parameters");
    if (ak == "<" && generic(pa)) || (bk == ">" && generic(pb)) {
        return (Zero, "generics");
    }
    if bk == "<" && pb == "type_arguments" && a.field == Some("base") {
        return (Zero, "generics");
    }
    if (ak == "::" && pa == "method_reference_expression")
        || (bk == "::" && pb == "method_reference_expression")
    {
        return (Zero, "member");
    }
    // prefix and postfix operators: `-x`, `!x`, `*args`, `i++`
    if UNARY.contains(&pa) && !a.node.is_named() && a.node.prev_sibling().is_none() {
        return if b.node.is_named() || bk == "(" {
            (Zero, "unary")
        } else {
            (Keep, "")
        };
    }
    if UNARY.contains(&pb) && !b.node.is_named() && b.node.next_sibling().is_none() {
        return if a.node.is_named() || matches!(ak, ")" | "]") {
            (Zero, "unary")
        } else {
            (Keep, "")
        };
    }
    // infix operators, assignment, ternary, `->`, `as`, `in`, `instanceof`
    let op = if is_op(b) {
        Some(b)
    } else if is_op(a) {
        Some(a)
    } else {
        None
    };
    if let Some(op) = op {
        if matches!(op.parent, "binary_expression" | "power_expression") && lopsided(op.node, src) {
            return (Keep, "");
        }
        if matches!(op.node.kind(), "<" | ">")
            && op.parent == "binary_expression"
            && generic_like(op.node)
        {
            return (Keep, "");
        }
        return (One, "operator");
    }
    // map entries, named arguments, labels, case labels: `a: 1`, `case 1:`
    if bk == ":" && matches!(pb, "map_entry" | "named_argument") {
        return (Zero, "colon");
    }
    if bk == ":" && matches!(pb, "labeled_statement" | "switch_case" | "switch_default") {
        return (Zero, "label");
    }
    if ak == ":" && matches!(pa, "map_entry" | "named_argument") {
        return (One, "colon");
    }
    // `(String) x`
    if ak == ")" && pa == "parenthesized_type_cast" {
        return (One, "cast");
    }
    // `if (`, `for (`, `while (`, `switch (`, `catch (`, `synchronized (`
    if bk == "("
        && pa == pb
        && matches!(
            ak,
            "if" | "for" | "while" | "switch" | "catch" | "synchronized"
        )
    {
        return (One, "keyword");
    }
    // `foo(x)`, `new Foo(x)`, `@Foo(x)`, `def foo(x)`
    if bk == "(" && pb == "argument_list" && anc.len() >= 3 {
        let (call, field, _, _) = anc[anc.len() - 2];
        let ctx = anc[anc.len() - 3].0.kind();
        let ok = match call.kind() {
            "annotation" => true,
            "method_invocation" | "object_creation_expression" => {
                CALL_CONTEXTS.contains(&ctx)
                    || (ctx == "assignment_expression" && field == Some("right"))
            }
            _ => false,
        };
        // the name ends the callee: not `(byte[]) (x)`, a cast read as a call
        let named = a.node.is_named() || ak == ">";
        return if ok && named && a.node.start_byte() >= call.start_byte() {
            (Zero, "call")
        } else {
            (Keep, "")
        };
    }
    if bk == "(" && pb == "formal_parameters" && a.field == Some("name") {
        return (Zero, "call");
    }
    // braces: `) {`, `{ x }`, `} else`. Nested braces (`{{`, `}}`) and a closure that
    // only makes sense as template text (`${x}` outside a string, a closure standing alone
    // as a statement, as in `{{name}}` placeholders) stay as written.
    if bk == "{" && BODIES.contains(&pb) {
        return if ak == "{" || b.odd {
            (Keep, "")
        } else {
            (One, "brace")
        };
    }
    if ak == "{" && BODIES.contains(&pa) {
        return if matches!(bk, "{" | "}") || a.odd {
            (Keep, "")
        } else {
            (One, "brace")
        };
    }
    if bk == "}" && BODIES.contains(&pb) {
        return if ak == "}" || b.odd {
            (Keep, "")
        } else {
            (One, "brace")
        };
    }
    if ak == "}" && BODIES.contains(&pa) && matches!(bk, "else" | "catch" | "finally" | "while") {
        return (One, "brace");
    }
    (Collapse, "collapse")
}

/// The lines a token on `lines[i]` (indented by `indent`) may be aligned with: the lines just
/// above and below, and, within 20 lines and until a line indented less, the lines indented
/// the same way (map entries aligned across nested values, siblings across a blank line).
fn neighbors(lines: &[(String, &str)], i: usize, indent: &str) -> Vec<usize> {
    let mut v: Vec<usize> = [i.checked_sub(1), Some(i + 1)]
        .into_iter()
        .flatten()
        .filter(|&j| j < lines.len())
        .collect();
    let mut siblings = |it: &mut dyn Iterator<Item = usize>| {
        for j in it {
            let l = &lines[j].0;
            let body = l.trim_start();
            if body.is_empty() || body.starts_with("//") {
                continue;
            }
            if l.len() - body.len() < indent.len() {
                break;
            }
            if l.starts_with(indent) && l.len() - body.len() == indent.len() {
                v.push(j);
            }
        }
    };
    siblings(&mut (i.saturating_sub(20)..i).rev());
    siblings(&mut (i + 1..lines.len().min(i + 21)));
    v
}

/// A respaced line, the last gap that is an aligned run of spaces before code, if any, and
/// for each token after a blank that moved: (gap, old column, new column, first char).
type Respaced = (String, Option<usize>, Vec<(usize, usize, usize, char)>);

/// Lines longer than this many bytes are left as re-indented. Alignment is looked up by
/// column, so every line's characters are indexed once per sweep; with lines this short,
/// and columns looked up no further than `COLS`, a sweep is linear in the file.
const MAX_LINE: usize = 1000;
const COLS: usize = 2 * MAX_LINE;
/// See `respace`.
const MAX_SWEEPS: usize = 64;

/// The first `COLS` characters of `s`, for lookups by column.
fn cols(s: &str) -> Vec<char> {
    s.chars().take(COLS).collect()
}

/// Whether `line` has `ch` at column `col`, not continuing a run of `ch`.
fn same_at(line: &[char], col: usize, ch: char) -> bool {
    line.get(col) == Some(&ch) && (col == 0 || line[col - 1] != ch)
}

/// Whether `line` has `ch` at column `col` after a blank: a token lined up there.
fn aligned_at(line: &[char], col: usize, ch: char) -> bool {
    same_at(line, col, ch) && col > 0 && line[col - 1].is_whitespace()
}

/// Whether `line` has, at column `col`, something `ch` could be aligned with: a token after
/// a blank (`x  = 1` over `foo = 2`), or `ch` itself starting a token (`name   :` over
/// `version:`).
fn token_at(line: &[char], col: usize, ch: char) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    match (col.checked_sub(1).and_then(|p| line.get(p)), line.get(col)) {
        (_, None) => false,
        (_, Some(c)) if c.is_whitespace() => false,
        (None, Some(_)) => true,
        (Some(&p), Some(&c)) => p.is_whitespace() || (c == ch && p != c && !(word(p) && word(c))),
    }
}

/// Respace `lines` (the re-indented output, each with its terminator) in place. `bodies[i]`
/// is, for a line holding source text, the source offset of that text and the length of the
/// indent written before it; `None` for blank lines and lines kept verbatim. False, with
/// `lines` as they were, when the sweeps below don't settle.
///
/// Whether a run of spaces is alignment depends on the lines around it, which may change
/// too, so lines are respaced against their neighbors' current text until nothing changes:
/// formatting the result again then finds every kept run still aligned.
///
/// Termination: the state is the set of gaps that have been changed (`forced`) and where each
/// line is held (`held`); both only grow and are bounded by the number of gaps. A gap not
/// forced is either kept because it lines up, or forced in that sweep. So in a sweep that
/// changes no state every line's text is a function of the state alone, and the next sweep
/// reproduces it and stops: there are at most two sweeps per state change. In practice two
/// or three sweeps settle a file; if `MAX_SWEEPS` don't, the file is skipped rather than
/// written half-respaced.
pub(crate) fn respace(
    src: &str,
    tree: &Tree,
    lines: &mut [(String, &str)],
    bodies: &[Option<(usize, usize)>],
) -> bool {
    respace_capped(src, tree, lines, bodies, MAX_SWEEPS)
}

fn respace_capped(
    src: &str,
    tree: &Tree,
    lines: &mut [(String, &str)],
    bodies: &[Option<(usize, usize)>],
    max_sweeps: usize,
) -> bool {
    let gaps = gaps(src, tree);
    // (line, indent length, source offset of its text, its gaps) for lines that may change
    let mut todo = Vec::new();
    let mut g = 0;
    for (i, body) in bodies.iter().enumerate() {
        let Some((pos, pad)) = *body else { continue };
        let end = pos + lines[i].0.len() - pad;
        while g < gaps.len() && gaps[g].start < pos {
            g += 1;
        }
        let first = g;
        while g < gaps.len() && gaps[g].end <= end {
            g += 1;
        }
        let here = &gaps[first..g];
        if lines[i].0.len() <= MAX_LINE
            && here
                .iter()
                .any(|x| x.want != Want::Keep || x.end - x.start > 1)
        {
            todo.push((i, pad, pos, first..g));
        }
    }
    let original: Vec<String> = todo.iter().map(|t| lines[t.0].0.clone()).collect();
    let original_cols: Vec<Vec<char>> = original.iter().map(|s| cols(s)).collect();
    // indents don't change, so neither do the lines each line is compared with
    let near: Vec<Vec<usize>> = todo
        .iter()
        .map(|&(i, pad, ..)| neighbors(lines, i, &lines[i].0[..pad]))
        .collect();
    let mut needed = vec![false; lines.len()];
    for &j in near.iter().flatten() {
        needed[j] = true;
    }
    let mut of_line = vec![None; lines.len()];
    for (k, t) in todo.iter().enumerate() {
        of_line[t.0] = Some(k);
    }
    // A run kept for alignment can later lose what it lined up with; once a gap has been
    // changed it stays changed.
    let forced: Vec<Cell<bool>> = gaps.iter().map(|_| Cell::new(false)).collect();
    // A line laid out in columns (`[ 4,  4,  3]` over `[10,  9,  8]`) keeps its spacing up to
    // its last aligned run of code. A line whose changes would move a `=`, `:` or comment out
    // of line with the same token on a nearby line keeps its spacing up to that token, unless
    // that line moves the same way; once held, a line stays held, so a group of lines lined
    // up with each other either all change or all stay.
    // ponytail: other tokens that line up are coincidences in similar lines and don't count
    let mut held: Vec<Option<usize>> = vec![None; todo.len()];
    for _ in 0..max_sweeps {
        let grid: Vec<Vec<char>> = lines
            .iter()
            .zip(&needed)
            .map(|(l, &n)| if n { cols(&l.0) } else { Vec::new() })
            .collect();
        let line = |k: usize, freeze: usize| {
            let (_, pad, pos, ref r) = todo[k];
            let near: Vec<&[char]> = near[k].iter().map(|&j| grid[j].as_slice()).collect();
            let at = (original[k].as_str(), pad, pos);
            respace_line(
                src,
                at,
                (&gaps[r.clone()], &forced[r.clone()]),
                &near,
                freeze,
            )
        };
        // every line respaced on its own against the current text around it
        let free: Vec<Respaced> = (0..todo.len()).map(|k| line(k, 0)).collect();
        let free_cols: Vec<Vec<char>> = free
            .iter()
            .zip(&todo)
            .map(|(f, t)| if needed[t.0] { cols(&f.0) } else { Vec::new() })
            .collect();
        let was = held.clone();
        let texts: Vec<String> = free
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let breaks = |&(_, ocol, ncol, ch): &(usize, usize, usize, char)| {
                    matches!(ch, '=' | ':' | '/')
                        && near[k].iter().any(|&j| {
                            let (orig, next) = match of_line[j] {
                                Some(kj) if was[kj].is_some() => (&original_cols[kj], &grid[j]),
                                Some(kj) => (&original_cols[kj], &free_cols[kj]),
                                None => (&grid[j], &grid[j]),
                            };
                            aligned_at(orig, ocol, ch) && !same_at(next, ncol, ch)
                        })
                };
                let moved = f.2.iter().filter(|m| breaks(m)).map(|m| m.0).max();
                held[k] = held[k].max(moved);
                match held[k].max(f.1) {
                    Some(a) => line(k, a + 1).0,
                    None => f.0.clone(),
                }
            })
            .collect();
        let mut changed = held != was;
        for (k, text) in texts.into_iter().enumerate() {
            let i = todo[k].0;
            if text != lines[i].0 {
                lines[i].0 = text;
                changed = true;
            }
        }
        if !changed {
            return true;
        }
    }
    for (k, t) in todo.iter().enumerate() {
        lines[t.0].0.clone_from(&original[k]);
    }
    false
}

/// `line` (the re-indented text of a line, which starts at source offset `pos` after its
/// indent) with its gaps respaced, judging alignment against the current text of the lines
/// around it (`near`), except the first `freeze` gaps, which stay as they are.
fn respace_line(
    src: &str,
    (line, pad, pos): (&str, usize, usize),
    (here, forced): (&[Gap], &[Cell<bool>]),
    near: &[&[char]],
    freeze: usize,
) -> Respaced {
    let mut text = String::with_capacity(line.len() + 8);
    text.push_str(&line[..pad]);
    // columns in the output and in `line`
    let mut col = text.chars().count();
    let mut ocol = col;
    let (mut from, mut shift, mut anchor, mut moved) = (pos, 0isize, None, Vec::new());
    for (k, x) in here.iter().enumerate() {
        let before = &src[from..x.start];
        text.push_str(before);
        col += before.chars().count();
        ocol += before.chars().count();
        let gap = &src[x.start..x.end];
        let w = gap.len();
        let tab = gap.contains('\t');
        let ruled = match x.want {
            _ if tab || k < freeze => w,
            Want::Keep => w,
            Want::Zero => 0,
            Want::One => 1,
            Want::Collapse => w.min(1),
        };
        // a run of spaces that stays absorbs the shift from earlier changes on the line,
        // so what it aligns keeps its column
        let kept = if tab || w < 2 {
            w
        } else {
            (w as isize - shift).max(1) as usize
        };
        let ch = src[x.end..].chars().next().unwrap_or(' ');
        // `9 :` over `10:`: a single space can line up a colon too
        let aligned = ruled != w
            && !forced[k].get()
            && ((w == 1
                && ruled == 0
                && x.rule == "colon"
                && near.iter().any(|n| same_at(n, col + 1, ch)))
                || (kept >= 2 && near.iter().any(|n| token_at(n, col + kept, ch))));
        let to = if ruled == w || aligned { kept } else { ruled };
        if ruled != w && !aligned {
            forced[k].set(true);
        }
        shift += to as isize - w as isize;
        let comment = src[x.end..].starts_with("//") || src[x.end..].starts_with("/*");
        if to >= 2 && !comment {
            anchor = Some(k);
        }
        if w >= 1 && col + to != ocol + w {
            moved.push((k, ocol + w, col + to, ch));
        }
        ocol += w;
        if to == w {
            text.push_str(gap);
        } else {
            text.push_str(&" ".repeat(to));
        }
        col += to;
        from = x.end;
    }
    text.push_str(&src[from..pos + line.len() - pad]);
    (text, anchor, moved)
}

#[cfg(test)]
mod tests {
    use crate::format::{Outcome, Style, format};

    fn on() -> Style {
        Style {
            normalize_spacing: true,
            ..Style::default()
        }
    }

    /// Formatted with spacing on, checked idempotent.
    fn fmt(src: &str) -> String {
        let out = match format(src, &on()) {
            Outcome::Unchanged => src.to_string(),
            Outcome::Formatted(o) => o,
            Outcome::Skipped(why) => panic!("skipped: {why}"),
        };
        assert_eq!(
            format(&out, &on()),
            Outcome::Unchanged,
            "not idempotent: {out:?}"
        );
        out
    }

    /// Each case is (input, expected), as one file.
    fn check(cases: &[(&str, &str)]) {
        for (src, want) in cases {
            assert_eq!(fmt(&format!("{src}\n")), format!("{want}\n"), "{src}");
        }
    }

    #[test]
    fn commas_semicolons_parens_brackets() {
        check(&[
            ("f(a,b , c)", "f(a, b, c)"),
            ("f( a, b )", "f(a, b)"),
            ("def l = [ 1,2 ]", "def l = [1, 2]"),
            ("def m = [ : ]", "def m = [:]"),
            ("def t = [1, 2, ]", "def t = [1, 2, ]"),
            ("print(x) ;", "print(x);"),
            (
                "for (int i = 0 ;i < 3 ;i++) {}",
                "for (int i = 0; i < 3; i++) {}",
            ),
            ("for (;;) {}", "for (;;) {}"),
            ("while (x) ;", "while (x) ;"),
            ("String [] a = new String [3]", "String[] a = new String[3]"),
        ]);
    }

    #[test]
    fn operators() {
        check(&[
            ("x=a+b*c", "x = a + b * c"),
            ("x+=1", "x += 1"),
            ("def y=1", "def y = 1"),
            ("c = a?b:d", "c = a ? b : d"),
            ("e = a?:d", "e = a ?: d"),
            ("e = a??d", "e = a ?? d"),
            ("e ??=d", "e ??= d"),
            ("t = 2**3", "t = 2 ** 3"),
            ("x = a<=>b", "x = a <=> b"),
            ("x = a<b", "x = a < b"),
            ("ok = x  in  xs", "ok = x in xs"),
            ("k = z  as  String", "k = z as String"),
            ("i = x  instanceof  Y", "i = x instanceof Y"),
            ("def f(a=1) {}", "def f(a = 1) {}"),
            ("def lam = (a,b)->a+b", "def lam = (a, b) -> a + b"),
            ("xs.each{a,b->a}", "xs.each { a, b -> a }"),
            ("def c = {->1}", "def c = { -> 1 }"),
        ]);
    }

    #[test]
    fn unary_spread_member_access_and_ranges() {
        check(&[
            ("x = - y", "x = -y"),
            ("x = ! y", "x = !y"),
            ("i ++", "i++"),
            ("m = f( * args)", "m = f(*args)"),
            ("u = String :: valueOf", "u = String::valueOf"),
            // left as written
            ("n = list*.name", "n = list*.name"),
            ("o = p?.q", "o = p?.q"),
            ("p = foo.&bar", "p = foo.&bar"),
            ("r = 1..2", "r = 1..2"),
            ("r = 1 .. 2", "r = 1 .. 2"),
            ("s = 1..<n", "s = 1..<n"),
            ("x .y", "x .y"),
        ]);
    }

    #[test]
    fn colons_casts_generics_annotations() {
        check(&[
            ("def m = [a:1, b : 2]", "def m = [a: 1, b: 2]"),
            ("g(a:1, b :2)", "g(a: 1, b: 2)"),
            ("foo a:1", "foo a: 1"),
            ("lbl :\nfor (i in xs) {}", "lbl:\nfor (i in xs) {}"),
            (
                "switch (x) {\n    case 1 :\n        break\n    default :\n        break\n}",
                "switch (x) {\n    case 1:\n        break\n    default:\n        break\n}",
            ),
            ("def k = (String)z", "def k = (String) z"),
            ("List <String> v = []", "List<String> v = []"),
            (
                "Map<String,Integer> w = [:]",
                "Map<String, Integer> w = [:]",
            ),
            ("def zz = new Foo<Bar> ()", "def zz = new Foo<Bar>()"),
            ("class C< T > {}", "class C<T> {}"),
            ("def <T> T f() {}", "def <T> T f() {}"),
            ("@Ann (x=1)\ndef w() {}", "@Ann(x = 1)\ndef w() {}"),
        ]);
    }

    #[test]
    fn keywords_calls_and_braces() {
        check(&[
            ("if(x){y()}else{z()}", "if (x) { y() } else { z() }"),
            ("while(x) z()", "while (x) z()"),
            ("for(i in xs){}", "for (i in xs) {}"),
            ("switch(x){\n}", "switch (x) {\n}"),
            (
                "try{ a() }catch(E e){ b() }finally{ c() }",
                "try { a() } catch (E e) { b() } finally { c() }",
            ),
            ("synchronized(x) {}", "synchronized (x) {}"),
            // a map key spelled like a keyword is a key
            ("def m = [else:1, catch :2]", "def m = [else: 1, catch: 2]"),
            ("do { x() }while(y)", "do { x() } while (y)"),
            ("foo (x)", "foo(x)"),
            ("def a = foo (x)", "def a = foo(x)"),
            ("stage ('b') {\n}", "stage('b') {\n}"),
            (
                "def f2 ( String a , int b ){ return a }",
                "def f2(String a, int b) { return a }",
            ),
            ("f{ it }", "f { it }"),
            ("def  x  = 1", "def x = 1"),
            // a cast in parentheses before a parenthesized value isn't a call
            ("x = (byte[])  (y)", "x = (byte[]) (y)"),
            // template placeholders that happen to parse
            ("println {{bar}}", "println {{bar}}"),
            ("println ${bar}", "println ${bar}"),
            ("{includeTools}", "{includeTools}"),
        ]);
    }

    #[test]
    fn spacing_that_can_change_meaning_is_left_alone() {
        check(&[
            // to a reader, a command call with a negative argument
            ("a -b", "a -b"),
            ("println -1", "println -1"),
            // Groovy may read these as `foo((x).bar())` and `foo((x) + 1)`
            ("def a1 = foo (x).bar()", "def a1 = foo (x).bar()"),
            ("y = foo (x) + 1", "y = foo (x) + 1"),
            // a subscript, or a command call with a list
            ("a [1]", "a [1]"),
            ("ok = x !in y", "ok = x !in y"),
            ("ok = x  !in  y", "ok = x  !in  y"),
            // a generic type the grammar reads as `Kind < Path > []`
            ("y = x as Kind<Path>[]", "y = x as Kind<Path>[]"),
            // `b = 2 ->` parses differently from `b=2 ->`
            (
                "def c = { int a, int b=2 -> a+b }",
                "def c = { int a, int b=2 -> a + b }",
            ),
        ]);
    }

    #[test]
    fn strings_comments_and_multiline_tokens_are_untouched() {
        check(&[
            ("def s = \"a ${b+c}   d\"", "def s = \"a ${b+c}   d\""),
            ("f(a,b)// c", "f(a, b)// c"),
            ("f( /* x */a)", "f( /* x */a)"),
            (
                "def t = '''x  =  y\n  a  ,b'''  ,  q",
                "def t = '''x  =  y\n  a  ,b'''  ,  q",
            ),
            ("def s = '''a'''+'b'", "def s = '''a''' + 'b'"),
        ]);
    }

    #[test]
    fn alignment_is_kept() {
        let aligned = [
            // `=` columns
            "def x1   = 1\ndef yy22 = 2\n",
            // map keys and values
            "def m = [\n    name   : 'x',\n    version: '1',\n]\n",
            "def m = [\n    a:   1,\n    bb:  foo,\n]\n",
            // trailing comments
            "a()      // one\nbb(1)    // two\n",
            // Gradle dependency columns
            "dependencies {\n    compile     'a:b'\n    testCompile 'c:d'\n}\n",
            // the spacing before the last aligned column stays as it is
            "m[4]  == [ 4,  4,  3]\nm[10] == [10,  9,  8]\n",
            // a quoted key lined up with the next one's `:`
            "def m = [\n    'a'   : 1,\n    'bcde': 2,\n]\n",
            // a change would push the comment out of its column: the line stays
            "def fooAB(a,b) {2} // a\ndef fooABCDEFG = 1 // b\n",
            "Tt1  obj = new Tt1()  // a\nTtt2 obj = new Tt2()  // b\n",
            // a group lined up with a line that doesn't change stays as a whole
            "ff(a:1) // x\nff(b:2) // y\n\ng(c: 3) // z\n",
            // a single space lining up colons
            "def m = [\n    9 : 'a',\n    10: 'b',\n]\n",
            // keys aligned across nested values and a blank line
            "def m = [\n    a   : [\n        x: 1,\n    ],\n\n    bcd : [\n        y: 2,\n    ],\n]\n",
        ];
        for src in aligned {
            assert_eq!(fmt(src), src);
        }
        // a lone run isn't alignment
        assert_eq!(fmt("def z  = 3\n"), "def z = 3\n");
        // lines that change the same way keep their columns lined up
        assert_eq!(
            fmt("def fooAB(a,b) {2}   //-> case A\ndef fooAD(a,c) {2}   //-> case D\n"),
            "def fooAB(a, b) { 2 } //-> case A\ndef fooAD(a, c) { 2 } //-> case D\n"
        );
        // a change before an aligned comment keeps the comment's column
        assert_eq!(
            fmt("foo(a,b)    // one\nbar(c, d)   // two\n"),
            "foo(a, b)   // one\nbar(c, d)   // two\n"
        );
        // runs that only line up with each other as they change still settle, idempotently
        // (`fmt` checks): from an openboxes map whose keys were half aligned
        fmt(
            "def m = [\n    productCode  : a,\n    productName  : a,\n    displayName : a,\n    qtyOrderedNotShipped : a,\n    orderNumber  : a,\n    supplierLocation  : a,\n    shipmentNumber  : a,\n    shipDate  : a,\n    shipmentType  : a\n]\n",
        );
        // each run is judged on its own: the `=` column stays, the run after it goes
        assert_eq!(fmt("x  = 1\nyy = 2\nz  =  3\n"), "x  = 1\nyy = 2\nz  = 3\n");
    }

    #[test]
    fn spock_tables_and_where_blocks_are_untouched() {
        let src = "def 'f'() {\n    expect:\n    a == b\n    where:\n    a  |b\n    1  |2\n    x << [1,2]\n    c           ; d\n}\n";
        assert_eq!(fmt(src), src);
        // outside `where:`, `|` and `||` are operators like any other
        assert_eq!(
            fmt("def f() {\n    xs.findAll{a||b}\n}\n"),
            "def f() {\n    xs.findAll { a || b }\n}\n"
        );
        // before `where:` the block is formatted as usual
        assert_eq!(
            fmt("def 'f'() {\n    expect:\n    f(a,b)\n    where:\n    a|b\n}\n"),
            "def 'f'() {\n    expect:\n    f(a, b)\n    where:\n    a|b\n}\n"
        );
    }

    #[test]
    fn tabs_inside_a_line_are_kept() {
        assert_eq!(fmt("def x\t= 1\n"), "def x\t= 1\n");
    }

    #[test]
    fn sweeps_that_do_not_settle_leave_the_lines_as_they_were() {
        // any change takes two sweeps: one to make it, one to see nothing else moves
        let src = "f(a,b)\n";
        let tree = crate::parse(src);
        let mut lines = vec![("f(a,b)".to_string(), "\n")];
        let bodies = [Some((0, 0))];
        assert!(!super::respace_capped(src, &tree, &mut lines, &bodies, 1));
        assert_eq!(lines[0].0, "f(a,b)");
        assert!(super::respace_capped(src, &tree, &mut lines, &bodies, 2));
        assert_eq!(lines[0].0, "f(a, b)");
    }

    #[test]
    fn long_lines_and_many_aligned_runs_stay_linear() {
        // alignment is looked up by column; walking each line from its start per lookup was
        // quadratic in the line, times the sweeps
        let t = std::time::Instant::now();
        // over the line limit: left as re-indented
        let long = format!("x = [{}a:b]\n", "a:b,".repeat(50_000));
        assert_eq!(fmt(&long), long);
        let row = |pad: usize| {
            let r: String = (0..90)
                .map(|i| format!("v{i}{}  = {i}; ", "x".repeat(pad)))
                .collect();
            r.trim_end().to_string() + "\n"
        };
        // every run lines up with the rows around it: kept
        let aligned = row(0).repeat(300);
        assert_eq!(fmt(&aligned), aligned);
        // rows that line up only here and there: sweeps settle (`fmt` checks a second pass
        // changes nothing)
        let ragged: String = (0..300).map(|n| row(n % 3)).collect();
        fmt(&ragged);
        assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
    }

    #[test]
    fn off_by_default() {
        let src = "if(x){\n    foo (a,b)\n}\n";
        assert_eq!(format(src, &Style::default()), Outcome::Unchanged);
    }
}
