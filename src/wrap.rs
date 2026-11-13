//! `max_line_length`: wrapping lines that are too long. Off unless the config sets it. It
//! only ever turns the spaces between two tokens on one line into a line break: it never
//! joins lines, never breaks inside a token, and never touches a line inside a string or
//! comment, a Spock `where:` block, an annotation, an `import` or `package` line or a `case`
//! label.
//!
//! A break goes only where real Groovy reads the code the same with or without it. The ground
//! truth is Groovy's own Parrot grammar (`src/antlr/GroovyLexer.g4` and `GroovyParser.g4` in
//! apache/groovy):
//!
//! - Inside `(` … `)` and `[` … `]` (but not `try (`), the lexer drops newlines before the
//!   parser sees them: `NL : LineTerminator { ignoreTokenInsideParens(); }`, where
//!   `isInsideParens` is true when the innermost open bracket is `(` (not after `try`), `[` or
//!   `?[`. A break there leaves the token stream Groovy parses unchanged. Argument lists, list
//!   and map literals, a method's or constructor's parameters (`formalParameters : LPAREN
//!   formalParameterList? rparen`) and an annotation's arguments (`annotation : AT
//!   annotationName (nls LPAREN elementValues? rparen)?`) break only there: after the opening
//!   bracket, after each comma and before the closing bracket.
//! - After each comma between a parenthesis-free command call's arguments: `argumentList :
//!   firstArgumentListElement (COMMA nls argumentListElement)*`.
//! - Anywhere else (in a block or closure body, or at the top level) a newline is a token, so
//!   a break goes only where a rule accepts `NL*` at that point:
//!   - before the `.`, `?.` or `*.` of a method chain: `pathElement : NL* (DOT NL* … |
//!     (SPREAD_DOT | SAFE_DOT | SAFE_CHAIN_DOT) NL* …)`;
//!   - after `||`, `&&`, `==`, `!=` (`left=expression NL* op=… NL* right=expression`) and
//!     after `+` and `-` (`left=expression op=(ADD | SUB) NL* right=expression`: a newline may
//!     follow these two but not precede them, so the operator always ends the line);
//!   - after a ternary's `?` and `:` and after `?:` (`con=expression NL* (QUESTION NL*
//!     tb=expression NL* COLON NL* | ELVIS NL* …) fb=expression`);
//!   - after `=`, only when nothing else on the line can break (`left=expression NL*
//!     op=ASSIGN NL* right=…` and `variableDeclaratorId (NL* ASSIGN NL* variableInitializer)`);
//!   - in a class, interface, trait, enum or record header, before `extends`, `implements`
//!     and `permits` (`classDeclaration : … (NL* EXTENDS NL* typeList)? (NL* IMPLEMENTS NL*
//!     typeList)? (NL* PERMITS NL* typeList)? NL* classBody`) and after each comma of their
//!     types (`typeList : type (COMMA NL* type)*`).
//!
//!   None of these leaves a line starting with `{`, `++`, `--`, `+`, `-`, `*` or `/`.
//!
//! Nothing else breaks inside a parenthesis-free command call (`foo a, b`), where
//! tree-sitter-groovy finds the first argument by looking at newlines, or anything inside a
//! closure or block written on one line, whose statements end at newlines.
//!
//! Layout: for each line over the limit, the outermost construct that lies wholly on that line
//! is broken one way (an argument list or literal one item per line, its closing bracket on a
//! line of its own; a chain of three or more calls before each `.` but the first; a binary
//! expression after each operator of its lowest-precedence level), then the file is
//! re-indented and the lines measured again, until no line over the limit can be broken
//! further. Operators and `=` break only when every piece then fits. A line whose overflow is
//! only a trailing comment, or that holds a string too long to fit anyway, is left alone, and
//! so are constructs whose breaking would scatter a part of the line that can't break itself
//! (see `scattered` and `select`).
//!
//! Each pass parses the result again and requires the same tree shape as the input; `format`
//! checks the final text again (tokens and tree shape, on the last pass's parse of it). If
//! anything fails, the file is formatted without wrapping instead.

use crate::format::{Style, shape};
use crate::spacing::{is_atom, lopsided};
use std::cell::RefCell;
use tree_sitter::{Node, Tree};

/// Accepted values of `max_line_length`.
pub const WIDTHS: std::ops::RangeInclusive<usize> = 40..=400;

/// Passes before giving up: each pass breaks every long line once more, so this bounds how
/// deeply constructs nested on one line are broken. A file that needs more is formatted
/// without wrapping, so the result is always a fixed point.
const MAX_PASSES: usize = 16;

/// Why lines still over the limit were left as they are, indexing `Report::left`.
pub const LEFT: [&str; 6] = [
    "inside a string or comment",
    "only a trailing comment overflows",
    "a string too long to fit",
    "in an excluded context",
    "no safe break",
    "outside --lines",
];
const VERBATIM: usize = 0;
const COMMENT: usize = 1;
const STRING: usize = 2;
const EXCLUDED: usize = 3;
const NO_BREAK: usize = 4;
const OUTSIDE: usize = 5;

/// Kinds of break, indexing `Report::breaks`.
pub const KINDS: [&str; 6] = [
    "list",
    "chain",
    "operator",
    "assignment",
    "command",
    "header",
];
const LIST: usize = 0;
const CHAIN: usize = 1;
const OPERATOR: usize = 2;
const ASSIGN: usize = 3;
const COMMAND: usize = 4;
const HEADER: usize = 5;

/// What wrapping did to the last file formatted on this thread, for measuring it.
#[doc(hidden)]
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// Lines over the limit before and after wrapping.
    pub over_before: usize,
    pub over_after: usize,
    /// Line breaks added, by `KINDS`.
    pub breaks: [usize; 6],
    /// Lines left over the limit, by `LEFT`.
    pub left: [usize; 6],
    /// Why the file was formatted without wrapping, if it was.
    pub fallback: Option<&'static str>,
}

thread_local! {
    static LAST: RefCell<Report> = RefCell::default();
}

/// The report for the last file formatted with `max_line_length` on this thread.
#[doc(hidden)]
pub fn last_report() -> Report {
    LAST.with_borrow(Clone::clone)
}

/// Why the last file formatted on this thread was formatted without wrapping, if it was; the
/// report is cleared, so a later file that doesn't wrap at all doesn't inherit it.
pub fn take_fallback() -> Option<&'static str> {
    LAST.take().fallback
}

pub(crate) fn set_report(r: Report) {
    LAST.set(r);
}

pub(crate) fn set_fallback(why: &'static str) {
    LAST.with_borrow_mut(|r| r.fallback = Some(why));
}

/// A token: a leaf, or a whole string or comment.
#[derive(Clone, Copy)]
struct Atom {
    start: usize,
    end: usize,
    row: usize,
    end_row: usize,
    comment: bool,
    string: bool,
}

fn atoms(tree: &Tree) -> Vec<Atom> {
    let mut v = Vec::new();
    crate::preorder(tree.root_node(), |n| {
        if !is_atom(n) {
            return true;
        }
        if n.start_byte() < n.end_byte() {
            let k = n.kind();
            v.push(Atom {
                start: n.start_byte(),
                end: n.end_byte(),
                row: n.start_position().row,
                end_row: n.end_position().row,
                comment: k.contains("comment"),
                string: k.contains("string"),
            });
        }
        false
    });
    v
}

/// Columns `s` takes, a tab counting as `indent`.
fn width(s: &str, indent: usize) -> usize {
    s.chars().map(|c| if c == '\t' { indent } else { 1 }).sum()
}

/// `width(s)`, or more than `cap` if it is: looks at no more than `cap + 1` characters.
fn width_upto(s: &str, indent: usize, cap: usize) -> usize {
    let mut w = 0;
    for c in s.chars() {
        w += if c == '\t' { indent } else { 1 };
        if w > cap {
            break;
        }
    }
    w
}

/// Each line's terminator in `src`, by row; the last line gets `\n` if it has none.
fn eols(src: &str) -> Vec<&'static str> {
    let b = src.as_bytes();
    let mut v: Vec<&'static str> = memchr_lines(src)
        .map(|i| {
            if i > 0 && b[i - 1] == b'\r' {
                "\r\n"
            } else {
                "\n"
            }
        })
        .collect();
    v.push("\n");
    v
}

fn memchr_lines(s: &str) -> impl Iterator<Item = usize> + '_ {
    s.bytes()
        .enumerate()
        .filter(|&(_, c)| c == b'\n')
        .map(|(i, _)| i)
}

/// `src` with the spaces before each atom `k` with `breaks[k]` replaced by its line's ending.
fn insert(src: &str, at: &[Atom], breaks: &[bool], eols: &[&str]) -> String {
    let mut s = String::with_capacity(src.len() + src.len() / 8);
    let mut pos = 0;
    for (k, _) in breaks.iter().enumerate().filter(|(_, b)| **b) {
        s.push_str(&src[pos..at[k - 1].end]);
        s.push_str(eols[at[k].row]);
        pos = at[k].start;
    }
    s.push_str(&src[pos..]);
    s
}

/// `base` (the formatted `src`, and its tree) with long lines wrapped, and the tree of the
/// result, or why it couldn't be. `fmt` formats a text with the line breaks added (re-indenting
/// it, and respacing it if that is on). `rows`: with `--lines`, the source lines (1-based,
/// inclusive) that may be wrapped.
pub(crate) fn wrap(
    src: &str,
    tree: &Tree,
    (base, base_tree): (&str, &Tree),
    style: &Style,
    limit: usize,
    rows: Option<&[(usize, usize)]>,
    fmt: &dyn Fn(&str, &Tree) -> Result<String, &'static str>,
) -> Result<(String, Tree), &'static str> {
    let mut report = Report {
        over_before: base
            .split('\n')
            .filter(|l| width(l.strip_suffix('\r').unwrap_or(l), style.indent) > limit)
            .count(),
        ..Report::default()
    };
    if report.over_before == 0 {
        set_report(report);
        return Ok((base.to_string(), base_tree.clone()));
    }
    let r = passes(
        src,
        tree,
        (base, base_tree),
        style,
        limit,
        rows,
        fmt,
        &mut report,
    );
    report.fallback = r.as_ref().err().copied();
    if r.is_err() {
        report.over_after = report.over_before;
    }
    set_report(report);
    r
}

#[allow(clippy::too_many_arguments)]
fn passes(
    src: &str,
    tree: &Tree,
    (base, base_tree): (&str, &Tree),
    style: &Style,
    limit: usize,
    rows: Option<&[(usize, usize)]>,
    fmt: &dyn Fn(&str, &Tree) -> Result<String, &'static str>,
    report: &mut Report,
) -> Result<(String, Tree), &'static str> {
    if too_deep(tree) {
        return Err("nesting too deep to wrap");
    }
    let src_at = atoms(tree);
    let want = shape(tree);
    let eols = eols(src);
    let mut breaks = vec![false; src_at.len()];
    // `base` was parsed already, to check the formatting; each pass parses its text again
    let mut cur = base.to_string();
    let mut ct = base_tree.clone();
    for pass in 0..=MAX_PASSES {
        if !same_shape(&ct, &want) {
            return Err(if pass == 0 {
                "formatting changed the parse"
            } else {
                "wrapping changed the parse"
            });
        }
        let at = atoms(&ct);
        if at.len() != src_at.len() {
            return Err("wrapping changed the tokens");
        }
        let ctx = Ctx {
            cur: &cur,
            at: &at,
            src,
            src_at: &src_at,
            indent: style.indent,
            cont: style.continuation.unwrap_or(style.indent),
            limit,
            rows,
        };
        let pick = ctx.select(&ct);
        report.over_after = pick.over;
        report.left = pick.left;
        if pick.breaks.is_empty() {
            return Ok((cur, ct));
        }
        if pass == MAX_PASSES {
            break;
        }
        for (k, kind) in pick.breaks {
            if !breaks[k] {
                breaks[k] = true;
                report.breaks[kind] += 1;
            }
        }
        let text = insert(src, &src_at, &breaks, &eols);
        let tt = crate::parse(&text);
        if !same_shape(&tt, &want) {
            return Err("a line break changed the parse");
        }
        cur = fmt(&text, &tt)?;
        ct = crate::parse(&cur);
    }
    Err("wrapping did not settle")
}

/// Whether `t` parsed cleanly to `want`, a `shape`, without building its own.
fn same_shape(t: &Tree, want: &[(u16, usize)]) -> bool {
    let mut i = 0;
    let mut same = !t.root_node().has_error();
    crate::preorder(t.root_node(), |n| {
        same &= want.get(i) == Some(&(n.kind_id(), n.child_count()));
        i += 1;
        same
    });
    same && i == want.len()
}

/// Trees nested deeper than this aren't wrapped. Each pass parses the file again, and
/// tree-sitter takes several times longer on deep nesting; real code stays far below it.
const MAX_DEPTH: usize = 1000;

fn too_deep(tree: &Tree) -> bool {
    let mut c = tree.walk();
    let mut depth = 0;
    loop {
        if c.goto_first_child() {
            depth += 1;
            if depth > MAX_DEPTH {
                return true;
            }
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return false;
            }
            depth -= 1;
        }
    }
}

/// The breaks one pass adds, as (atom index, kind), and the lines over the limit.
struct Pick {
    breaks: Vec<(usize, usize)>,
    over: usize,
    left: [usize; 6],
}

struct Ctx<'a> {
    cur: &'a str,
    at: &'a [Atom],
    src: &'a str,
    src_at: &'a [Atom],
    indent: usize,
    cont: usize,
    limit: usize,
    rows: Option<&'a [(usize, usize)]>,
}

/// A node on the walk, its flags, whether a `where:` label came among its children so far, and
/// the row the statement before ended on and whether a `;` followed it.
type Frame<'t> = (Node<'t>, u8, bool, Option<(usize, bool)>);

/// Ancestor flags on the walk.
const SKIP: u8 = 1; // never break here
const CMD: u8 = 2; // in a parenthesis-free command call, outside any brackets
const IN_LIST: u8 = 4; // in an argument list or literal of two or more items on one line
const MID: u8 = 8; // in an operand of a one-line ternary or `?:` other than the last
const ANN: u8 = 16; // in an annotation: only its brackets break

/// Whether following `next` down from `n` (its first or last child, and so on) reaches a
/// parenthesis-free command call before a bracket. Gives up, saying yes, after `MAX_EDGE`
/// steps, so nested constructs on one line cost linear time each.
fn edge_command<'t>(n: Node<'t>, next: impl Fn(Node<'t>) -> Option<Node<'t>>) -> bool {
    let mut m = n;
    for _ in 0..MAX_EDGE {
        match m.kind() {
            "command_chain" => return true,
            "argument_list"
            | "list_literal"
            | "map_literal"
            | "parenthesized_expression"
            | "closure" => return false,
            _ => {}
        }
        match next(m) {
            Some(c) => m = c,
            None => return false,
        }
    }
    true
}

const MAX_EDGE: usize = 64;

fn last_child(n: Node) -> Option<Node> {
    n.child(u32::try_from(n.child_count().checked_sub(1)?).ok()?)
}

/// Whether breaking `n`, an operator expression or a chain, would scatter a part of a
/// larger construct on its line that can't break itself: an operator's operand that is
/// itself an operator expression (`(a != null && b) ? x : y`), a parenthesized part
/// (`(a ?: b).c()`), a call's only argument (`f(a + b)`), the condition of an `assert` with
/// a message, or the condition of an `if` or `while` whose body follows it on the line.
fn scattered(n: Node, parent: Option<Node>) -> bool {
    let Some(p) = parent else { return false };
    let operator = n.kind() != "method_invocation";
    // `f((T) a.b().c().d())`: as the only argument, through a cast
    let p = match p.kind() {
        "cast_expression" | "parenthesized_type_cast" => match p.parent() {
            Some(pp) if pp.kind() == "argument_list" => pp,
            _ => p,
        },
        _ => p,
    };
    match p.kind() {
        "binary_expression" | "ternary_expression" | "elvis_expression" => operator,
        // a lambda's body (`.filter(m -> m.x() == y)`), like a closure on one line, and a
        // subscript (`a[b + 1] = c`)
        "parenthesized_expression" | "lambda_expression" | "subscript_expression" => true,
        "argument_list" => only_on_one_line(p),
        // `f(key: a + b)`, the only argument too
        "named_argument" => p
            .parent()
            .is_some_and(|l| l.kind() == "argument_list" && only_on_one_line(l)),
        // `assert a == b : 'message'`: the message would trail the last piece
        "assert_statement" => p.child_by_field_name("message").is_some(),
        "if_statement" | "while_statement" => ["consequence", "body"]
            .iter()
            .filter_map(|f| p.child_by_field_name(f))
            .any(|b| {
                let row = b.start_position().row;
                row == n.end_position().row && (b.kind() != "block" || b.end_position().row == row)
            }),
        _ => false,
    }
}

/// Whether `n`, the only argument of a call, ends in brackets that break instead of the call's:
/// a list, map or closure, or a call whose own arguments are two or more or one that hugs.
fn hugs(n: Node) -> bool {
    let mut m = n;
    for _ in 0..MAX_EDGE {
        if matches!(
            m.kind(),
            "list_literal" | "map_literal" | "closure" | "lambda_expression"
        ) {
            return true;
        }
        let Some(l) = last_child(m).filter(|l| l.kind() == "argument_list") else {
            return false;
        };
        match (l.named_child_count(), l.named_child(0)) {
            (1, Some(only)) => m = only,
            (c, _) => return c >= 2,
        }
    }
    false
}

/// Whether `l`, an argument list, holds one argument and lies on one line: then that argument
/// breaking would leave the `)` trailing its last piece, and the list breaks first instead.
fn only_on_one_line(l: Node) -> bool {
    l.named_child_count() == 1 && l.start_position().row == l.end_position().row
}

/// Whether a method call is the last of its chain: not the object of a `.`, nor called.
fn chain_top(field: Option<&str>, parent: Option<Node>) -> bool {
    let inner = match field {
        Some("object") => parent.is_some_and(|p| {
            matches!(
                p.kind(),
                "field_access" | "safe_navigation_expression" | "spread_dot_expression"
            )
        }),
        Some("function") => parent.is_some_and(|p| p.kind() == "method_invocation"),
        _ => false,
    };
    !inner
}

/// Columns below which a piece an operator leaves is short, and too short to leave at all.
const SHORT: usize = 20;
const TINY: usize = 8;

/// A construct that can break, on one line.
struct Cand {
    /// assignments and only arguments come last, then operators leaving a short piece; then
    /// the widest
    rank: (bool, bool, usize),
    kind: usize,
    breaks: Vec<usize>,
}

impl Ctx<'_> {
    fn text(&self, a: &Atom) -> &str {
        &self.cur[a.start..a.end]
    }

    /// The atom starting at `byte`.
    fn idx(&self, byte: usize) -> Option<usize> {
        self.at.binary_search_by_key(&byte, |a| a.start).ok()
    }

    fn in_rows(&self, src_row: usize) -> bool {
        self.rows
            .is_none_or(|r| r.iter().any(|&(a, b)| a <= src_row + 1 && src_row < b))
    }

    /// Whether the gap before atom `k` is spaces on one line, in the text and in the source,
    /// and on a line `--lines` allows.
    fn gap_ok(&self, k: usize) -> bool {
        let blank = |s: &str| s.bytes().all(|b| b == b' ' || b == b'\t');
        k > 0
            && k < self.at.len()
            && self.at[k - 1].end_row == self.at[k].row
            && blank(&self.cur[self.at[k - 1].end..self.at[k].start])
            && self.src_at[k - 1].end_row == self.src_at[k].row
            && blank(&self.src[self.src_at[k - 1].end..self.src_at[k].start])
            && self.in_rows(self.src_at[k].row)
    }

    fn select(&self, tree: &Tree) -> Pick {
        let cur = self.cur;
        let mut starts = vec![0];
        starts.extend(memchr_lines(cur).map(|i| i + 1));
        let nrows = starts.len();
        let line = |r: usize| {
            let end = starts.get(r + 1).map_or(cur.len(), |e| e - 1);
            let l = &cur[starts[r]..end];
            l.strip_suffix('\r').unwrap_or(l)
        };
        let mut pick = Pick {
            breaks: Vec::new(),
            over: 0,
            left: [0; 6],
        };
        let over: Vec<usize> = (0..nrows)
            .filter(|&r| width(line(r), self.indent) > self.limit)
            .collect();
        pick.over = over.len();
        if over.is_empty() {
            return pick;
        }
        // rows that start inside a multi-line token, and the atoms starting on each row
        let mut verbatim = vec![false; nrows];
        let mut on_row = vec![(0usize, 0usize); nrows];
        for (k, a) in self.at.iter().enumerate() {
            for v in &mut verbatim[a.row + 1..=a.end_row] {
                *v = true;
            }
            if on_row[a.row].1 == 0 {
                on_row[a.row].0 = k;
            }
            on_row[a.row].1 = k + 1;
        }
        let mut want = vec![false; nrows];
        for &r in &over {
            let (first, last) = on_row[r];
            let l = line(r);
            let lead = width(&l[..l.len() - l.trim_start().len()], self.indent);
            let why = if verbatim[r] || first == last {
                Some(VERBATIM)
            } else if !self.in_rows(self.src_at[first].row) {
                Some(OUTSIDE)
            } else if self.at[last - 1].comment
                && width(
                    cur[starts[r]..self.at[last - 1].start].trim_end(),
                    self.indent,
                ) <= self.limit
            {
                Some(COMMENT)
            } else if self.at[first..last].iter().any(|a| {
                let t = self.text(a);
                a.string
                    && lead + self.cont + width(t.split('\n').next().unwrap_or(t), self.indent)
                        > self.limit
            }) {
                Some(STRING)
            } else {
                None
            };
            match why {
                Some(w) => pick.left[w] += 1,
                None => want[r] = true,
            }
        }
        let mut pre = vec![0usize; nrows + 1];
        for r in 0..nrows {
            pre[r + 1] = pre[r] + usize::from(want[r]);
        }
        if pre[nrows] == 0 {
            return pick;
        }
        // For the gap before each atom: whether Groovy's lexer drops a newline there (the
        // innermost open bracket is `(` but not `try (`, `[` or `?[`), and whether it is
        // inside a `{` opened on the same line (a one-line closure or block, never broken).
        let mut hidden = vec![(false, false); self.at.len()];
        let mut open: Vec<(bool, Option<usize>)> = Vec::new();
        for (k, a) in self.at.iter().enumerate() {
            let (drop, brace) = open.last().copied().unwrap_or((false, None));
            hidden[k] = (drop, brace == Some(a.row));
            match self.text(a) {
                "(" => open.push((k == 0 || self.text(&self.at[k - 1]) != "try", brace)),
                "[" | "?[" => open.push((true, brace)),
                "{" | "${" => open.push((false, Some(a.row))),
                ")" | "]" | "}" => {
                    open.pop();
                }
                _ => {}
            }
        }
        let mut best: Vec<Option<Cand>> = (0..nrows).map(|_| None).collect();
        let mut excluded = vec![false; nrows];
        let mut crowded = vec![false; nrows];
        // the walk: each frame is a node, its flags, whether a `where:` label came among its
        // children so far, and the row the statement before ended on and whether a `;`
        // followed it
        let mut c = tree.walk();
        let mut stack: Vec<Frame> = Vec::new();
        'walk: loop {
            let n = c.node();
            let (sr, er) = (n.start_position().row, n.end_position().row);
            let mut flags = stack.last().map_or(0, |f| f.1);
            if let Some(f) = stack.last_mut() {
                if n.kind() == "labeled_statement"
                    && n.child_by_field_name("label")
                        .is_some_and(|l| crate::text(l, cur) == "where")
                {
                    f.2 = true;
                }
                if f.2 {
                    flags |= SKIP;
                }
                if matches!(f.0.kind(), "switch_case" | "switch_arrow_case")
                    && c.field_name().is_some()
                {
                    flags |= SKIP;
                }
                // `c ? f(\n a,\n b\n) : d`: what follows would trail the closing bracket
                if matches!(f.0.kind(), "ternary_expression" | "elvis_expression")
                    && f.0.start_position().row == f.0.end_position().row
                    && matches!(c.field_name(), Some("condition" | "consequence" | "value"))
                {
                    flags |= MID;
                }
                // A line holding two statements is left alone. Without a `;` between them it
                // is a command call the grammar split in two (`println a + b` read as
                // `println` and `a + b`); with one, the second would trail the last piece.
                let k = n.kind();
                if n.is_named() && (k.ends_with("statement") || k.ends_with("declaration")) {
                    if let Some((row, semi)) = f.3
                        && row == sr
                    {
                        crowded[sr] = true;
                        if !semi {
                            flags |= SKIP;
                        }
                    }
                    f.3 = Some((er, false));
                } else if k == ";"
                    && let Some(p) = &mut f.3
                {
                    p.1 = true;
                }
            }
            // a command call's own arguments may break, but not those of one inside another's
            let outer_cmd = flags & CMD;
            match n.kind() {
                "closure" | "block" | "class_body" | "enum_body" | "switch_block" => flags &= !CMD,
                "command_chain" => flags |= CMD,
                "annotation" => flags |= ANN,
                "import_declaration" | "package_declaration" => flags |= SKIP,
                _ => {}
            }
            // `if (c) x = f(a, b)`: a body without braces on its statement's line is left
            // alone, as two statements on a line are
            if matches!(
                n.kind(),
                "if_statement" | "while_statement" | "for_statement" | "for_in_statement"
            ) {
                let mut c2 = n.walk();
                for (i, k) in n.children(&mut c2).enumerate() {
                    let f = u32::try_from(i)
                        .ok()
                        .and_then(|i| n.field_name_for_child(i));
                    if matches!(f, Some("consequence" | "alternative" | "body"))
                        && !matches!(k.kind(), "block" | "if_statement")
                    {
                        let r = k.start_position().row;
                        let prev = k.prev_sibling().map_or(sr, |p| p.end_position().row);
                        if r == prev && r < nrows {
                            crowded[r] = true;
                        }
                    }
                }
            }
            let listish = matches!(n.kind(), "argument_list" | "list_literal" | "map_literal");
            let in_list = sr == er && listish && n.named_child_count() >= 2;
            let live = er < nrows && pre[er + 1] > pre[sr];
            if live && listish && sr < er {
                // a list over several lines with two or more items on a line (laid out by
                // hand): that line is left as it is
                let mut c2 = n.walk();
                let mut last = None;
                for k in n.named_children(&mut c2) {
                    let r = k.start_position().row;
                    if last == Some(r) && !k.kind().contains("comment") {
                        crowded[r] = true;
                    }
                    last = Some(r);
                }
            }
            let mut blocked = false;
            if live && !is_atom(n) {
                let parent = stack.last().map(|f| f.0);
                let mut found = Vec::new();
                if matches!(
                    n.kind(),
                    "class_declaration"
                        | "interface_declaration"
                        | "trait_declaration"
                        | "enum_declaration"
                        | "record_declaration"
                ) {
                    for (r, breaks) in self.header(n, &on_row) {
                        if want[r] {
                            found.push((r, Ok(Some((HEADER, breaks)))));
                        }
                    }
                } else if sr == er {
                    found.push((sr, self.candidate(n, c.field_name(), parent)));
                } else if n.kind() == "method_invocation" && chain_top(c.field_name(), parent) {
                    // a chain already over several lines breaks before its dots on this one
                    for (r, breaks) in self.chain_rows(n, &on_row) {
                        if want[r] {
                            found.push((r, Ok(Some((CHAIN, breaks)))));
                        }
                    }
                }
                for (r, cand) in found {
                    let checked = cand.and_then(|c| match c {
                        Some((kind, breaks)) => {
                            let fl = if kind == COMMAND {
                                (flags & !CMD) | outer_cmd
                            } else {
                                flags
                            };
                            self.check(n, kind, breaks, fl, &hidden, (starts[r], line(r).len()))
                        }
                        None => Ok(None),
                    });
                    match checked {
                        Ok(Some(cand)) => {
                            if best[r].as_ref().is_none_or(|b| cand.rank > b.rank) {
                                best[r] = Some(cand);
                            }
                        }
                        Ok(None) => {}
                        Err(()) => {
                            excluded[r] = true;
                            // what may not break here doesn't scatter its parts either
                            blocked = sr == er;
                        }
                    }
                }
                if c.goto_first_child() {
                    let more = if in_list { IN_LIST } else { 0 } | if blocked { SKIP } else { 0 };
                    stack.push((n, flags | more, false, None));
                    continue;
                }
            }
            loop {
                if c.goto_next_sibling() {
                    continue 'walk;
                }
                if !c.goto_parent() {
                    break 'walk;
                }
                stack.pop();
            }
        }
        for r in 0..nrows {
            if !want[r] {
                continue;
            }
            // `=` is the last resort: not on a line where something else may not break
            match best[r].take() {
                _ if crowded[r] => pick.left[EXCLUDED] += 1,
                Some(cand) if !(cand.kind == ASSIGN && excluded[r]) => pick
                    .breaks
                    .extend(cand.breaks.iter().map(|&k| (k, cand.kind))),
                _ => pick.left[if excluded[r] { EXCLUDED } else { NO_BREAK }] += 1,
            }
        }
        pick
    }

    /// The kind of break and the breaks for `n`, a node on one line: `Ok(None)` if it isn't
    /// a construct that breaks, `Err` if it is one but may not break here.
    fn candidate(
        &self,
        n: Node,
        field: Option<&str>,
        parent: Option<Node>,
    ) -> Result<Option<(usize, Vec<usize>)>, ()> {
        let (kind, breaks) = match n.kind() {
            "argument_list" | "list_literal" | "map_literal" => (LIST, self.items(n)),
            // a method's or constructor's parameters, two or more
            "formal_parameters"
                if parent.is_some_and(|p| {
                    matches!(p.kind(), "method_declaration" | "constructor_declaration")
                }) =>
            {
                (LIST, self.items(n).filter(|b| b.len() > 2))
            }
            "method_invocation" if chain_top(field, parent) => (
                CHAIN,
                self.chain(n)
                    .map(|b| if scattered(n, parent) { Vec::new() } else { b }),
            ),
            "binary_expression" | "ternary_expression" | "elvis_expression" => {
                (OPERATOR, self.operators(n, parent))
            }
            "assignment_expression" | "variable_declarator" => (ASSIGN, self.assign(n)),
            "command_chain" => (COMMAND, self.command_args(n)),
            _ => return Ok(None),
        };
        match breaks {
            None => Ok(None),
            Some(b) if b.is_empty() => Err(()),
            Some(b) => Ok(Some((kind, b))),
        }
    }

    /// `breaks` for `n`, if they may go where they are; `line` is where their line starts.
    fn check(
        &self,
        n: Node,
        kind: usize,
        breaks: Vec<usize>,
        flags: u8,
        hidden: &[(bool, bool)],
        (line, len): (usize, usize),
    ) -> Result<Option<Cand>, ()> {
        // `println a + b` may parse as `(println a) + b`: a command call at either edge of an
        // operator, chain or assignment puts the break inside it
        if !matches!(kind, LIST | COMMAND | HEADER) && edge_command(n, |m| m.child(0)) {
            return Err(());
        }
        if !matches!(kind, LIST | COMMAND | HEADER) && edge_command(n, last_child) {
            // `def r = sh a: 1, b: 2`: the `=` stays, the command's own arguments may break
            return if kind == ASSIGN { Ok(None) } else { Err(()) };
        }
        // A command call's arguments break only where nothing follows the call on its line
        // but a comment, and when the line up to the first break then fits: `foo "a long
        // string",` then `x` keeps the long line and only moves the short part.
        if kind == COMMAND {
            let rest = self.cur[n.end_byte()..line + len].trim_start();
            let first = &self.cur[line..self.at[breaks[0] - 1].end];
            if !(rest.is_empty() || rest.starts_with("//") || rest.starts_with("/*"))
                || width_upto(first, self.indent, self.limit) > self.limit
            {
                return Ok(None);
            }
        }
        // a list holding this on one line couldn't break (it holds a comment): breaking
        // inside one of its items instead would scatter it
        if flags & (SKIP | MID) != 0 || (kind != LIST && flags & IN_LIST != 0) {
            return Err(());
        }
        // in an annotation only brackets break, but an `x = [a, b]` around one doesn't stop it
        if kind != LIST && flags & ANN != 0 {
            return Ok(None);
        }
        for &k in &breaks {
            // tree-sitter-groovy reads `f(x, List<T>` then a line break as `List < T >`; in a
            // class header a `>` closes a type, as in `class A<T>` then `extends B<T>`
            if !self.gap_ok(k) || (kind != HEADER && self.text(&self.at[k - 1]) == ">") {
                return Ok(None);
            }
            let (drop, one_line_brace) = hidden[k];
            // lists break only where the lexer drops newlines, the others also where the
            // parser takes them; nothing breaks inside a closure or block written on one line
            // or a parenthesis-free command call (`input "a", "b", options: [x, y]`)
            if one_line_brace || flags & CMD != 0 || (!drop && kind == LIST) {
                return Err(());
            }
            // Outside brackets, no line starts with `{`, `++`, `--`, `+`, `-`, `*` or `/`: even
            // after a line Groovy must continue, it reads like a new statement or a closure.
            let next = self.text(&self.at[k]);
            if !drop && next.starts_with(['{', '+', '-', '*', '/']) && !next.starts_with("*.") {
                return Ok(None);
            }
        }
        // An operator or `=` whose pieces wouldn't all fit isn't broken: `a.b().c().d() == x`
        // breaks the chain and `x == [a, b, c]` the list instead.
        // A list followed on its line by more than it holds (`f(_, _) >> { long closure ->`,
        // `g(a, b).x.y.z == long`) gains little from breaking and reads worse broken.
        if kind == LIST {
            let (o, z) = (breaks[0] - 1, breaks[breaks.len() - 1]);
            let inside = width_upto(
                &self.cur[self.at[o].end..self.at[z].start],
                self.indent,
                self.limit,
            );
            let after = width_upto(
                &self.cur[self.at[z].end..line + len],
                self.indent,
                self.limit,
            );
            if after > inside {
                return Ok(None);
            }
            // An annotation's arguments break only with nothing after them on the line
            // (`@A(x, y) def f()` stays). One item breaks only if it is long enough to be worth
            // a line of its own and at least half as wide as what comes before it on the line
            // (a long `a.b(c).d(` then a short `x` then `)` reads worse than the long line),
            // when it then fits on its own line or is an operator expression or a chain that
            // breaks further, and when only closing brackets, `,` or `;` follow it on the line
            // (not `) == 1`, nor `)) {` of an `if`, which would line up with the body).
            let single = breaks.len() == 2;
            let before = width_upto(&self.cur[line..self.at[o].end], self.indent, self.limit);
            let text = &self.cur[line..line + len];
            let lead = width(&text[..text.len() - text.trim_start().len()], self.indent);
            let item = n.named_child(0).map(|i| match i.kind() {
                "named_argument" => i.child_by_field_name("value").unwrap_or(i),
                _ => i,
            });
            let further = item.is_some_and(|i| match i.kind() {
                "binary_expression" => i
                    .child_by_field_name("operator")
                    .is_some_and(|o| matches!(o.kind(), "||" | "&&" | "==" | "!=" | "+" | "-")),
                "ternary_expression" | "elvis_expression" => true,
                "method_invocation" => self.chain(i).is_some(),
                _ => false,
            });
            if (after > 0 && n.parent().is_some_and(|p| p.kind() == "annotation"))
                || (single
                    && (inside < SHORT
                        || !self.cur[self.at[z].end..line + len]
                            .chars()
                            .all(|c| matches!(c, ')' | ']' | ',' | ';' | ' '))
                        || before > self.limit
                        || before.saturating_sub(lead) > 2 * inside
                        || (lead + self.cont + inside > self.limit && !further)))
            {
                return Ok(None);
            }
        }
        // One whose later pieces include a short one (`x.a().b().c() !=` then `"MyTx") {`)
        // comes after the other constructs.
        let (mut fits, mut stub, mut tiny) = (true, false, false);
        if matches!(kind, OPERATOR | ASSIGN) {
            let text = &self.cur[line..line + len];
            let lead = width(&text[..text.len() - text.trim_start().len()], self.indent);
            let mut from = line;
            for (i, &k) in breaks.iter().chain([&self.at.len()]).enumerate() {
                let end = if k < self.at.len() {
                    self.at[k - 1].end
                } else {
                    line + len
                };
                let w = width_upto(&self.cur[from..end], self.indent, self.limit);
                let pad = if i == 0 { 0 } else { lead + self.cont };
                fits &= pad + w <= self.limit;
                stub |= i > 0 && w < SHORT;
                // `x.y(z) ==` then `1`: one binary operator doesn't leave a piece this short
                tiny |= breaks.len() == 1 && i > 0 && w < TINY && n.kind() == "binary_expression";
                from = self.at.get(k).map_or(end, |a| a.start);
            }
        }
        // `=` and operators only when that makes every piece fit
        if !fits || tiny {
            return Ok(None);
        }
        // A call's only argument breaking ranks with `=`, so the wider `=` wins when both can:
        // `def x =` then the whole call, not `def x = a.b().c(` then a short argument.
        let last = kind == ASSIGN || (kind == LIST && breaks.len() == 2);
        Ok(Some(Cand {
            rank: (!last, !stub, n.end_byte() - n.start_byte()),
            kind,
            breaks,
        }))
    }

    /// An argument list, list or map literal or parameter list with no comments: after the
    /// opening bracket, after each comma, before the closing bracket.
    fn items(&self, n: Node) -> Option<Vec<usize>> {
        let count = n.child_count();
        let last = u32::try_from(count.checked_sub(1)?).ok()?;
        let (open, close) = (n.child(0)?, n.child(last)?);
        if !matches!(open.kind(), "(" | "[") || !matches!(close.kind(), ")" | "]") {
            return None;
        }
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        if kids.iter().any(|k| k.kind().contains("comment")) {
            return None;
        }
        let mut items = kids.iter().filter(|k| k.is_named());
        match (items.next(), items.next()) {
            (None, _) => return None,
            // one item: a list, map or closure breaks inside itself instead (`f([` then its
            // items, `])`)
            // and so does a call ending in its own arguments that break that way (`f(g(` then
            // them, `))`, `f(g(h([` then its items, `])))`)
            (Some(i), None) if hugs(*i) => return None,
            _ => {}
        }
        let (o, z) = (self.idx(open.start_byte())?, self.idx(close.start_byte())?);
        let mut v = vec![o + 1];
        for k in kids.iter().filter(|k| k.kind() == ",") {
            let next = self.idx(k.start_byte())? + 1;
            if next < z {
                v.push(next);
            }
        }
        v.push(z);
        Some(v)
    }

    /// A chain of three or more calls (`a.b().c().d()`, `foo(x).bar().baz()`): before the
    /// `.` of each call but the first. Only the calls at the end of a chain count; a
    /// property between them (`a.b().c.d()`) ends it.
    fn chain(&self, n: Node) -> Option<Vec<usize>> {
        let (mut links, head_call) = self.links(n)?;
        if links.len() + usize::from(head_call) < 3 {
            return None;
        }
        if !head_call {
            links.pop();
        }
        links.reverse();
        Some(links)
    }

    /// For a chain of three or more calls over several lines, by line: before each of its
    /// dots on that line, but the first call's and one starting the line.
    fn chain_rows(&self, n: Node, on_row: &[(usize, usize)]) -> Vec<(usize, Vec<usize>)> {
        let Some((mut links, head_call)) = self.links(n) else {
            return Vec::new();
        };
        if links.len() + usize::from(head_call) < 3 {
            return Vec::new();
        }
        if !head_call {
            links.pop();
        }
        links.reverse();
        let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
        for k in links {
            let r = self.at[k].row;
            if on_row[r].0 == k {
                continue;
            }
            match out.last_mut() {
                Some((row, v)) if *row == r => v.push(k),
                _ => out.push((r, vec![k])),
            }
        }
        out
    }

    /// The dots of the calls ending a chain, last call first, and whether the chain starts
    /// with a call (`foo(x).bar()`, `new Foo().bar()`).
    fn links(&self, n: Node) -> Option<(Vec<usize>, bool)> {
        let mut links = Vec::new(); // the dots, last call first
        let mut m = n;
        let head_call = loop {
            if m.kind() != "method_invocation" {
                break m.kind() == "object_creation_expression";
            }
            let f = m.child_by_field_name("function")?;
            if !matches!(
                f.kind(),
                "field_access" | "safe_navigation_expression" | "spread_dot_expression"
            ) {
                break true;
            }
            let mut c = f.walk();
            let dot = f
                .children(&mut c)
                .find(|d| matches!(d.kind(), "." | "?." | "*."))?;
            links.push(self.idx(dot.start_byte())?);
            m = f.child_by_field_name("object")?;
        };
        Some((links, head_call))
    }

    /// The operators of a binary expression's lowest-precedence level (`||`, `&&`, `==` and
    /// `!=`, `+` and `-`), a ternary's `?` and `:`, or `?:`: after each.
    fn operators(&self, n: Node, parent: Option<Node>) -> Option<Vec<usize>> {
        let after = |op: Node| self.idx(op.start_byte()).map(|k| k + 1);
        if n.kind() != "binary_expression" {
            if scattered(n, parent) {
                return Some(Vec::new());
            }
            let mut c = n.walk();
            let ops: Option<Vec<usize>> = n
                .children(&mut c)
                .filter(|k| matches!(k.kind(), "?" | ":" | "?:"))
                .map(after)
                .collect();
            return ops.filter(|o| !o.is_empty());
        }
        let group = |e: Node| {
            if e.kind() != "binary_expression" {
                return None;
            }
            match e.child_by_field_name("operator")?.kind() {
                "||" => Some(1),
                "&&" => Some(2),
                "==" | "!=" => Some(3),
                "+" | "-" => Some(4),
                _ => None,
            }
        };
        let g = group(n)?;
        if parent.and_then(group) == Some(g) {
            return None; // not the top of its level
        }
        if scattered(n, parent) {
            return Some(Vec::new());
        }
        let mut ops = Vec::new();
        let mut todo = vec![n];
        while let Some(e) = todo.pop() {
            if group(e) != Some(g) {
                continue;
            }
            let op = e.child_by_field_name("operator")?;
            // `a -b` reads as a command call with a negative argument; left alone
            if lopsided(op, self.cur) {
                return Some(Vec::new());
            }
            ops.push(after(op)?);
            todo.extend(e.child_by_field_name("left"));
            todo.extend(e.child_by_field_name("right"));
        }
        ops.sort_unstable();
        Some(ops)
    }

    /// A parenthesis-free command call with two or more arguments (`foo a, b`): after each
    /// comma between its arguments.
    fn command_args(&self, n: Node) -> Option<Vec<usize>> {
        let mut c = n.walk();
        let v: Option<Vec<usize>> = n
            .children(&mut c)
            .filter(|k| k.kind() == ",")
            .map(|k| self.idx(k.start_byte()).map(|i| i + 1))
            .collect();
        v.filter(|v| !v.is_empty())
    }

    /// The header of `n`, a class, interface, trait, enum or record declaration, by line:
    /// before each `extends`, `implements` or `permits` that doesn't start its line, or on a
    /// line starting with one that holds all of its clause, after each comma of the clause.
    /// Nothing for a header holding a comment, followed on its line by more of the body than
    /// `{}`, or of a record without a body.
    fn header(&self, n: Node, on_row: &[(usize, usize)]) -> Vec<(usize, Vec<usize>)> {
        let Some(body) = n.child_by_field_name("body") else {
            return Vec::new();
        };
        // from the name: a comment after an annotation on a line above doesn't count
        let name = n.child_by_field_name("name");
        let (Some(first), Some(open)) = (
            name.and_then(|m| self.idx(m.start_byte())),
            self.idx(body.start_byte()),
        ) else {
            return Vec::new();
        };
        if self.at[first..open].iter().any(|a| a.comment) {
            return Vec::new();
        }
        // `class A extends B {}` may break, `class A extends B { def x }` may not
        if let Some(next) = self.at.get(open + 1)
            && next.row == self.at[open].row
            && !(self.text(next) == "}" && next.end == body.end_byte())
        {
            return Vec::new();
        }
        let mut rows: Vec<(usize, Vec<usize>, Vec<usize>)> = Vec::new();
        let mut c = n.walk();
        for clause in n.named_children(&mut c).filter(|k| {
            matches!(
                k.kind(),
                "superclass" | "super_interfaces" | "extends_interfaces" | "permits_clause"
            )
        }) {
            let Some(kw) = self.idx(clause.start_byte()) else {
                return Vec::new();
            };
            let r = self.at[kw].row;
            let i = match rows.iter().position(|e| e.0 == r) {
                Some(i) => i,
                None => {
                    rows.push((r, Vec::new(), Vec::new()));
                    rows.len() - 1
                }
            };
            if on_row[r].0 != kw {
                rows[i].1.push(kw);
            } else if clause.end_position().row == r {
                let mut c2 = clause.walk();
                for comma in clause.children(&mut c2).filter(|k| k.kind() == ",") {
                    let Some(k) = self.idx(comma.start_byte()) else {
                        return Vec::new();
                    };
                    rows[i].2.push(k + 1);
                }
            }
        }
        rows.into_iter()
            .filter_map(|(r, kw, commas)| {
                if kw.is_empty() {
                    return (!commas.is_empty()).then_some((r, commas));
                }
                // as few keywords as leave the first piece fitting: `class A extends B` then
                // `implements C`, or before each when none does
                let from = self.at[kw[0]].start;
                let line = self.cur[..from].rfind('\n').map_or(0, |i| i + 1);
                let fits = |k: usize| {
                    width_upto(&self.cur[line..self.at[k - 1].end], self.indent, self.limit)
                        <= self.limit
                };
                let j = (0..kw.len()).rev().find(|&j| fits(kw[j])).unwrap_or(0);
                Some((r, kw[j..].to_vec()))
            })
            .collect()
    }

    /// `x = …` and `def x = …`: after the `=`.
    fn assign(&self, n: Node) -> Option<Vec<usize>> {
        let mut c = n.walk();
        let eq = n.children(&mut c).find(|k| k.kind() == "=")?;
        Some(vec![self.idx(eq.start_byte())? + 1])
    }
}

#[cfg(test)]
mod tests {
    use crate::format::{Outcome, Style, format};

    fn at(limit: usize) -> Style {
        Style {
            max_line_length: Some(limit),
            ..Style::default()
        }
    }

    fn no_ws(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Formatted at `style`, checked to be a fixed point and to change only whitespace.
    fn fmt_with(src: &str, style: &Style) -> String {
        let out = match format(src, style) {
            Outcome::Unchanged => src.to_string(),
            Outcome::Formatted(o) => o,
            Outcome::Skipped(why) => panic!("skipped: {why}"),
        };
        assert_eq!(no_ws(&out), no_ws(src), "{out}");
        assert_eq!(
            format(&out, style),
            Outcome::Unchanged,
            "not a fixed point: {out}"
        );
        out
    }

    fn fmt(src: &str) -> String {
        fmt_with(src, &at(40))
    }

    #[test]
    fn argument_lists_and_literals_one_item_per_line() {
        assert_eq!(
            fmt("foo(alpha, beta, gamma, delta, epsilon, zeta)\n"),
            "foo(\n    alpha,\n    beta,\n    gamma,\n    delta,\n    epsilon,\n    zeta\n)\n"
        );
        assert_eq!(
            fmt("def l = [alpha, beta, gamma, delta, epsilon]\n"),
            "def l = [\n    alpha,\n    beta,\n    gamma,\n    delta,\n    epsilon\n]\n"
        );
        assert_eq!(
            fmt("def m = [alpha: 1, beta: 2, gamma: 3, delta: 4]\n"),
            "def m = [\n    alpha: 1,\n    beta: 2,\n    gamma: 3,\n    delta: 4\n]\n"
        );
        // a trailing comma stays, and the closing bracket gets its own line once
        assert_eq!(
            fmt("def l = [alpha, beta, gamma, delta, epsilon,]\n"),
            "def l = [\n    alpha,\n    beta,\n    gamma,\n    delta,\n    epsilon,\n]\n"
        );
        // a trailing closure stays on the closing parenthesis' line
        assert_eq!(
            fmt("task(alpha, beta, gamma, delta, epsilon) {\n    x()\n}\n"),
            "task(\n    alpha,\n    beta,\n    gamma,\n    delta,\n    epsilon\n) {\n    x()\n}\n"
        );
        // nested: the outermost first, then whatever is still too long
        assert_eq!(
            fmt("foo(alpha, bar(beta, gamma, delta, epsilon))\n"),
            "foo(\n    alpha,\n    bar(beta, gamma, delta, epsilon)\n)\n"
        );
        assert_eq!(
            fmt("foo(alpha, bar(beta, gamma, delta, epsilon, zeta, eta, theta))\n"),
            "foo(\n    alpha,\n    bar(\n        beta,\n        gamma,\n        delta,\n        epsilon,\n        zeta,\n        eta,\n        theta\n    )\n)\n"
        );
        // one item: the list inside it breaks, hugged
        assert_eq!(
            fmt("foo([alpha, beta, gamma, delta, epsilon])\n"),
            "foo([\n    alpha,\n    beta,\n    gamma,\n    delta,\n    epsilon\n])\n"
        );
    }

    #[test]
    fn chains_break_before_each_call_but_the_first() {
        assert_eq!(
            fmt("def x = Foo.builder().name(alpha).value(beta).build()\n"),
            "def x = Foo.builder()\n    .name(alpha)\n    .value(beta)\n    .build()\n"
        );
        assert_eq!(
            fmt("def x = foo(alpha).bar(beta)?.baz(gamma)*.qux()\n"),
            "def x = foo(alpha)\n    .bar(beta)\n    ?.baz(gamma)\n    *.qux()\n"
        );
        // a chain over several lines breaks before its dots on a line still too long
        assert_eq!(
            fmt("def x = foo.bar().where(alpha).orderBy(beta).cache()\n    .first()\n"),
            "def x = foo.bar()\n    .where(alpha)\n    .orderBy(beta)\n    .cache()\n    .first()\n"
        );
        // a line already starting with one of a chain's dots breaks before the others
        assert_eq!(
            fmt("def x = foo\n    .where(alpha).orderBy(beta).cache().first()\n"),
            "def x = foo\n    .where(alpha)\n    .orderBy(beta)\n    .cache()\n    .first()\n"
        );
        // an operator whose first piece wouldn't fit gives way to the chain
        assert_eq!(
            fmt("ce.findIt(m).get().getReturnType().getArgs(List) == numberType\n"),
            "ce.findIt(m)\n    .get()\n    .getReturnType()\n    .getArgs(List) == numberType\n"
        );
        // two calls aren't a chain: the arguments break instead
        assert_eq!(
            fmt("def x = something.callIt(alpha, beta, gamma)\n"),
            "def x = something.callIt(\n    alpha,\n    beta,\n    gamma\n)\n"
        );
    }

    #[test]
    fn binary_operators_break_after_the_lowest_precedence_level() {
        assert_eq!(
            fmt("if (alpha && beta || gamma && delta || epsilon) {\n    x()\n}\n"),
            "if (alpha && beta ||\n    gamma && delta ||\n    epsilon) {\n    x()\n}\n"
        );
        assert_eq!(
            fmt("def s = alpha + beta + gamma + delta - epsilon\n"),
            "def s = alpha +\n    beta +\n    gamma +\n    delta -\n    epsilon\n"
        );
        assert_eq!(
            fmt("def s = condition ? alphaValue : betaValue123\n"),
            "def s = condition ?\n    alphaValue :\n    betaValue123\n"
        );
        assert_eq!(
            fmt("def s = firstChoiceValue ?: secondChoiceValue\n"),
            "def s = firstChoiceValue ?:\n    secondChoiceValue\n"
        );
        // an operator that would leave a short piece gives way to a chain
        assert_eq!(
            fmt("if (ops.find(alpha).get().getDefinition().getName() != 'MyTx') {\n}\n"),
            "if (ops.find(alpha)\n    .get()\n    .getDefinition()\n    .getName() != 'MyTx') {\n}\n"
        );
        // an operator whose pieces wouldn't all fit gives way to what is inside them
        assert_eq!(
            fmt("assert attributes == [alphaAlpha, betaBeta, gammaGamma, deltaDelta]\n"),
            "assert attributes == [\n    alphaAlpha,\n    betaBeta,\n    gammaGamma,\n    deltaDelta\n]\n"
        );
        // statements in a method body
        assert_eq!(
            fmt("def f() {\n    return alphaAlpha == betaBeta && gammaDelta\n}\n"),
            "def f() {\n    return alphaAlpha == betaBeta &&\n        gammaDelta\n}\n"
        );
    }

    #[test]
    fn assignment_breaks_only_when_nothing_else_can() {
        assert_eq!(
            fmt("def someVariable = alpha.beta.gamma.delta.epsilon\n"),
            "def someVariable =\n    alpha.beta.gamma.delta.epsilon\n"
        );
        assert_eq!(
            fmt("someVariable = alpha.beta.gamma.delta.epsilon\n"),
            "someVariable =\n    alpha.beta.gamma.delta.epsilon\n"
        );
    }

    #[test]
    fn parameters_annotations_and_command_arguments_break_one_per_line() {
        // a method's or constructor's parameters
        assert_eq!(
            fmt("class A {\n    String f(String alpha, int beta, long gamma) {\n    }\n}\n"),
            "class A {\n    String f(\n        String alpha,\n        int beta,\n        long gamma\n    ) {\n    }\n}\n"
        );
        assert_eq!(
            fmt("class A {\n    A(String alpha, int beta, long gammaGamma) {\n    }\n}\n"),
            "class A {\n    A(\n        String alpha,\n        int beta,\n        long gammaGamma\n    ) {\n    }\n}\n"
        );
        // an annotation's arguments, and a list in them
        assert_eq!(
            fmt("@Ann(alpha = 1, beta = 2, gamma = 3, delta = 4)\nclass A {\n}\n"),
            "@Ann(\n    alpha = 1,\n    beta = 2,\n    gamma = 3,\n    delta = 4\n)\nclass A {\n}\n"
        );
        assert_eq!(
            fmt("@SuppressWarnings(['alpha', 'beta', 'gamma', 'delta'])\ndef x\n"),
            "@SuppressWarnings([\n    'alpha',\n    'beta',\n    'gamma',\n    'delta'\n])\ndef x\n"
        );
        assert_eq!(
            fmt("@Ann(value = [alphaAlpha, betaBeta, gammaGamma], x = 1)\ndef x\n"),
            "@Ann(\n    value = [\n        alphaAlpha,\n        betaBeta,\n        gammaGamma\n    ],\n    x = 1\n)\ndef x\n"
        );
        // a parenthesis-free command call's arguments, after each comma
        assert_eq!(
            fmt("implementation group: 'org.example', name: 'artifact', version: '1.0'\n"),
            "implementation group: 'org.example',\n    name: 'artifact',\n    version: '1.0'\n"
        );
        assert_eq!(
            fmt("def r = sh returnStdout: true, script: 'make all'\n"),
            "def r = sh returnStdout: true,\n    script: 'make all'\n"
        );
        assert_eq!(
            fmt("input 'a', 'b', options: [alpha, beta, gamma, delta]\n"),
            "input 'a',\n    'b',\n    options: [alpha, beta, gamma, delta]\n"
        );
        assert_eq!(
            fmt("foo alpha.bar().baz().qux().quux(), x, y, z, w\n"),
            "foo alpha.bar().baz().qux().quux(),\n    x,\n    y,\n    z,\n    w\n"
        );
    }

    #[test]
    fn class_headers_break_before_extends_and_implements_then_after_commas() {
        let at60 = |s: &str| fmt_with(s, &at(60));
        assert_eq!(
            at60("class FooSpec extends Specification implements GrailsUnitTest, DataTest {\n}\n"),
            "class FooSpec extends Specification\n    implements GrailsUnitTest, DataTest {\n}\n"
        );
        // before both, when the first line would still be too long; after a `>` too
        assert_eq!(
            at60(
                "class FooSpec<T> extends AbstractSpecificationWithLongName<T> implements Testing {\n}\n"
            ),
            "class FooSpec<T>\n    extends AbstractSpecificationWithLongName<T>\n    implements Testing {\n}\n"
        );
        // one type per line when a clause still doesn't fit on its own line
        assert_eq!(
            at60(
                "class FooSpec extends Specification implements GrailsUnitTest, DataTest, ServiceUnitTest<FooService> {\n}\n"
            ),
            "class FooSpec extends Specification\n    implements GrailsUnitTest,\n        DataTest,\n        ServiceUnitTest<FooService> {\n}\n"
        );
        // interfaces, traits, enums and records, an annotation before it, a nested class and
        // an empty body on the line
        assert_eq!(
            at60("interface Something extends FirstInterface, SecondInterfaceName {\n}\n"),
            "interface Something\n    extends FirstInterface, SecondInterfaceName {\n}\n"
        );
        assert_eq!(
            at60("trait Something extends FirstInterface implements SecondInterface {\n}\n"),
            "trait Something extends FirstInterface\n    implements SecondInterface {\n}\n"
        );
        assert_eq!(
            at60("enum Something implements FirstInterface, SecondInterfaceName {\n    A, B\n}\n"),
            "enum Something\n    implements FirstInterface, SecondInterfaceName {\n    A, B\n}\n"
        );
        assert_eq!(
            at60("record Point(int x, int y) implements FirstInterface, Second {\n}\n"),
            "record Point(int x, int y)\n    implements FirstInterface, Second {\n}\n"
        );
        assert_eq!(
            at60("@CompileStatic class FooSpec extends Specification implements X {\n}\n"),
            "@CompileStatic class FooSpec extends Specification\n    implements X {\n}\n"
        );
        assert_eq!(
            at60(
                "class A {\n    static class Inner extends SomeLongBaseClass implements Other {\n    }\n}\n"
            ),
            "class A {\n    static class Inner extends SomeLongBaseClass\n        implements Other {\n    }\n}\n"
        );
        // a comment on an annotation's line above doesn't hold it back
        assert_eq!(
            at60(
                "@Ann // why\nclass FooSpec extends Specification implements GrailsUnitTest, X {\n}\n"
            ),
            "@Ann // why\nclass FooSpec extends Specification\n    implements GrailsUnitTest, X {\n}\n"
        );
        assert_eq!(
            at60("class SomeLongException extends SomeOtherLongBaseException {}\n"),
            "class SomeLongException\n    extends SomeOtherLongBaseException {}\n"
        );
        // left alone: a body on the line, a comment in the header
        for src in [
            "class FooSpec extends Specification implements GrailsUnit { def x }\n",
            "class FooSpec extends Specification /* x */ implements GrailsUnit {\n}\n",
        ] {
            assert_eq!(at60(src), src);
        }
    }

    #[test]
    fn a_long_only_argument_breaks_inside_its_parentheses() {
        assert_eq!(
            fmt("throw new IllegalStateException('it went wrong: ' + x)\n"),
            "throw new IllegalStateException(\n    'it went wrong: ' + x\n)\n"
        );
        // then what is inside breaks too, if it must
        assert_eq!(
            fmt("render(alphaAlphaAlphaAlpha ? betaBetaBeta : gamma)\n"),
            "render(\n    alphaAlphaAlphaAlpha ?\n        betaBetaBeta :\n        gamma\n)\n"
        );
        assert_eq!(
            fmt("write(key: alphaAlphaAlpha + betaBetaBetaBeta)\n"),
            "write(\n    key: alphaAlphaAlpha +\n        betaBetaBetaBeta\n)\n"
        );
        assert_eq!(
            fmt("foo(alpha.beta().gamma().delta().epsilon(zeta))\n"),
            "foo(\n    alpha.beta()\n        .gamma()\n        .delta()\n        .epsilon(zeta)\n)\n"
        );
        // `=` first, where it makes everything fit
        assert_eq!(
            fmt("def resultValue = f.resolve('alpha.beta.gamma.d')\n"),
            "def resultValue =\n    f.resolve('alpha.beta.gamma.d')\n"
        );
        // a call or a literal as the only argument breaks inside itself instead, also through
        // calls with one argument
        assert_eq!(
            fmt("foo(bar(alpha, beta, gamma, delta, epsilon))\n"),
            "foo(bar(\n    alpha,\n    beta,\n    gamma,\n    delta,\n    epsilon\n))\n"
        );
        assert_eq!(
            fmt("json(send(toJson([alpha, beta, gamma, delta])))\n"),
            "json(send(toJson([\n    alpha,\n    beta,\n    gamma,\n    delta\n])))\n"
        );
    }

    #[test]
    fn never_breaks_where_groovy_or_the_grammar_might_read_it_differently() {
        let same = [
            // operators and chains in a parenthesis-free command call's arguments, a command
            // call inside another's arguments, and one followed on its line by more
            "println alphaAlpha + betaBeta + gammaGamma + deltaDelta\n",
            "foo alpha.bar().baz().qux().quux().quuz().corge()\n",
            "list.each { put alphaAlpha, betaBeta, gammaGamma }\n",
            "foo alphaAlpha, betaBeta, gammaGamma; bar(d)\n",
            // a command call whose first piece would still be too long
            "foo 'a string long enough to overflow it', x\n",
            // after `return`, `throw`, `assert`
            "def f() {\n    return alphaAlphaAlpha.betaBetaBeta.gammaGamma\n}\n",
            "throw alphaAlphaAlpha.betaBetaBeta.gammaGammaGam\n",
            "assert alphaAlphaAlpha.betaBetaBeta.gammaGammaGam\n",
            // an annotation followed on its line, and what is in an annotation but its
            // brackets; imports, packages, case labels
            "@Ann(alpha = 1, beta = 2, gamma = 3) def x = 1\n",
            "@Ann(value = alphaAlphaAlpha + betaBetaBetaBeta)\ndef x\n",
            "import org.example.something.very.longer.pkg.Name\n",
            "package org.example.something.very.longer.pkg.names\n",
            "switch (x) {\n    case [alpha, beta, gamma, delta, epsilon]:\n        break\n}\n",
            // strings, GString interpolations, comments
            "def s = \"${foo(alpha, beta, gamma, delta, epsilon)}\"\n",
            "def s = 'a string that is far too long to fit in forty'\n",
            "foo(a, b) // a trailing comment that makes it too long\n",
            "foo(alpha, /* note */ beta, gamma, delta, epsilon)\n",
            "def s = '''\nfoo(alpha, beta, gamma, delta, epsilon, zeta, eta)\n'''\n",
            // a closure or block written on one line
            "list.each { foo(alpha, beta, gamma, delta, epsilon) }\n",
            "list.each { x -> alphaAlpha && betaBeta || gammaGamma }\n",
            "if (x) { foo(alpha, beta, gamma, delta, epsilon, zz) }\n",
            // Spock data tables
            "def 'f'() {\n    expect:\n    a == b\n    where:\n    a | b\n    [alpha, beta, gamma, delta, epsilon] | 1\n}\n",
            // one argument with nothing inside it to break that wouldn't fit on its own
            // line either, or that is short
            "foo(alphaAlphaAlpha.betaBetaBeta.gammaGammaGamma)\n",
            "alphaAlphaAlpha.betaBetaBeta.gammaGamma.foo(x)\n",
            // or that is less than half as wide as what comes before it, or followed by `{`
            "alphaAlphaAlpha.betaBeta(gamma).deltaEpsilon('twenty chars here!!')\n",
            "if (line.contains('a string of twenty-five chars')) {\n}\n",
            // `a -b`: to a reader, a command call with a negative argument
            "def x = alphaAlphaAlpha -betaBetaBeta -gammaGamma\n",
            // an operator inside parentheses that are part of a larger expression
            "def s = (alphaAlphaAlpha ?: betaBetaBetaBe).toString()\n",
            // a list followed on its line by more than it holds
            "1 * source.visit(_, _) >> { Visitor alpha, Tree beta ->\n}\n",
            // inside a one-line ternary's condition or first branch, a subscript, or a cast
            // that is a call's only argument
            "def x = alpha ? foo(beta, gamma, delta, epsilon, zeta) : z\n",
            "names[alphaAlphaAlpha + betaBetaBetaBeta + gammaG] = 1\n",
            "refs.add((Ref) loader.load(alpha).getCtor().newInstance())\n",
            // a body without braces on its statement's line
            "if (alphaAlpha) gammaGamma = foo(alpha, beta, gamma)\n",
            "if (x) {\n} else gammaGamma = foo(alpha, beta, gamma, d)\n",
            // a binary operator leaving a tiny piece, and the condition of an `if` whose
            // one-line block follows
            "output.count('alphaAlphaAlphaBetaBeta') == 1\n",
            "if (alphaAlpha != null) { betaBeta = gamma() }\n",
            // `=` or an operator followed by a closure or a prefix operator
            "alphaAlphaAlpha.betaBeta = { gamma -> gamma }\n",
            "assert alphaAlphaAlphaBetaGamma == -deltaDeltaDelta\n",
            // two statements on a line
            "foo(alpha, beta, gamma, delta); bar(epsilon)\n",
            // a list laid out by hand with several items on a line
            "foo(alpha,\n    beta, gamma, delta, epsilon, zeta, eta, theta)\n",
            // what would scatter a part that can't break: an operator inside another
            // operator's operand, the condition of an `if` whose body follows on the line
            "def q = (alphaAlpha != null && betaBeta) ? gamma : d\n",
            "if (alphaAlpha == betaBetaBeta) gammaGamma = delta\n",
            "assert alphaAlphaAlpha == betaBetaBeta : 'message'\n",
            // `=`, when what follows it still wouldn't fit
            "def x = alpha.beta.gamma.delta.epsilon.zeta.eta.theta\n",
            // `<`, `>`, `as`, `instanceof`: not break points
            "foo(alphaAlphaAlpha < betaBetaBeta < gammaGamma)\n",
            "foo(alphaAlphaAlpha as Map<String, List<Integer>>)\n",
            "foo(alphaAlphaAlpha instanceof BetaBetaBetaGamma)\n",
        ];
        for src in same {
            assert_eq!(fmt(src), src, "{src}");
        }
    }

    #[test]
    fn no_line_starts_with_an_opener_or_an_operator() {
        // breaks go after `(`, `[`, `,` and operators, before `)` or `]` whose opener broke,
        // and before a `.`: never between a name and its `(`, a call and its closure, or
        // before `{`, `[`, `++`, `--`, `+`, `-`
        let src =
            "def x = foo(alpha, beta)[gamma].bar { delta }.baz(epsilon).qux(zeta, eta) - theta\n";
        let out = fmt(src);
        assert_ne!(out, src);
        for line in out.lines().skip(1) {
            let t = line.trim_start();
            assert!(
                !["(", "[", "{", "++", "--", "+", "-"]
                    .iter()
                    .any(|p| t.starts_with(p)),
                "{out}"
            );
        }
    }

    #[test]
    fn lines_left_long_are_counted() {
        let src = "def s = 'a string that is far too long to fit in forty'\nfoo(a, b) // a trailing comment that makes it too long\n";
        assert_eq!(fmt(src), src);
        format(src, &at(40));
        let r = super::last_report();
        assert_eq!((r.over_before, r.over_after, r.fallback), (2, 2, None));
        assert_eq!(r.left[super::STRING], 1);
        assert_eq!(r.left[super::COMMENT], 1);
    }

    #[test]
    fn absent_means_no_wrapping() {
        let src = "foo(alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda, mu, nu, xi)\n";
        assert_eq!(format(src, &Style::default()), Outcome::Unchanged);
    }

    #[test]
    fn tabs_count_as_indent_columns_and_characters_not_bytes() {
        let tabs = Style {
            use_tabs: true,
            ..at(40)
        };
        // 31 columns of text after two tabs (8 columns): 39, fits
        let src = "class A {\n\tdef f() {\n\t\tfoo(alphaaa, beta, gamma, delta)\n\t}\n}\n";
        assert_eq!(fmt_with(src, &tabs), src);
        // with 8-wide tabs it doesn't
        let wide = Style { indent: 8, ..tabs };
        assert_ne!(fmt_with(src, &wide), src);
        // 39 characters, more bytes
        let src = "foo(\"ééééééééé\", \"éééééééééééééééééé\")\n";
        assert!(src.len() > 40);
        assert_eq!(fmt(src), src);
    }

    #[test]
    fn crlf_lines_keep_their_ending() {
        let src = "x = 1\r\nfoo(alpha, beta, gamma, delta, epsilon, zeta)\r\n";
        assert_eq!(
            fmt(src),
            "x = 1\r\nfoo(\r\n    alpha,\r\n    beta,\r\n    gamma,\r\n    delta,\r\n    epsilon,\r\n    zeta\r\n)\r\n"
        );
    }

    #[test]
    fn fixed_point_for_every_style_combination() {
        let src = "class A {\n    def f() {\n        def x = Foo.builder().name(alpha).value(computeIt(a, b, c)).build()\n        if (someCondition && otherCondition || yetAnotherCondition && last) {\n            def m = [alpha: 1, beta: 2, gamma: [3, 4, 5], delta: foo(x,y, z)]\n            render(view: 'show', model: [item: item, others: others], status: 200)\n            return a ? computeTheFirstThing(argumentOne) : second(argumentTwo)\n        }\n    }\n}\n";
        for limit in [40, 60, 100] {
            for indent in [2, 4] {
                for continuation in [None, Some(8)] {
                    for use_tabs in [false, true] {
                        for normalize_spacing in [false, true] {
                            let style = Style {
                                indent,
                                continuation,
                                use_tabs,
                                normalize_spacing,
                                indent_after_labels: false,
                                max_line_length: Some(limit),
                            };
                            let out = fmt_with(src, &style);
                            format(src, &style);
                            assert_eq!(super::last_report().fallback, None, "{style:?}");
                            if limit < 100 {
                                assert!(out.lines().count() > src.lines().count(), "{style:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn adversarial_input_stays_linear() {
        use crate::scaling::assert_scales;
        // a line of n nested calls, too deep to wrap: formatted without wrapping
        let deep = |n: usize| {
            let deep = format!("{}b{}\n", "f(a, ".repeat(n), ")".repeat(n));
            let _ = format(&deep, &at(100));
            assert_eq!(
                super::last_report().fallback,
                Some("nesting too deep to wrap")
            );
        };
        assert_scales("nesting too deep", deep, 2_000, 4, 8);
        // n lines nested deeper than the passes can finish: formatted without wrapping
        let unsettled = |n: usize| {
            let line = format!("{}b{}\n", "f(alpha, ".repeat(200), ")".repeat(200));
            assert_eq!(format(&line.repeat(n), &at(40)), Outcome::Unchanged);
            assert_eq!(
                super::last_report().fallback,
                Some("wrapping did not settle")
            );
        };
        assert_scales("passes that don't finish", unsettled, 1, 4, 8);
        // a flat argument list: one pass
        let flat = |n: usize| {
            let flat = format!("f({}x)\n", "alpha, ".repeat(n));
            assert_eq!(fmt_with(&flat, &at(100)).lines().count(), n + 3);
        };
        assert_scales("flat list", flat, 1_000, 4, 8);
        // a command call's arguments and a parameter list: one pass each
        let command = |n: usize| {
            let src = format!("foo {}x\n", "alpha, ".repeat(n));
            assert_eq!(fmt_with(&src, &at(100)).lines().count(), n + 1);
        };
        assert_scales("command arguments", command, 1_000, 4, 8);
        let params = |n: usize| {
            let src = format!("def f({}int x) {{\n}}\n", "int alpha, ".repeat(n));
            assert_eq!(fmt_with(&src, &at(100)).lines().count(), n + 4);
        };
        assert_scales("parameters", params, 1_000, 4, 8);
        // long chains
        let chains = |n: usize| {
            let chains =
                "def x = alpha.beta(gamma).delta(epsilon).zeta(eta).theta(iota)\n".repeat(n);
            assert_eq!(fmt_with(&chains, &at(40)).lines().count(), 4 * n);
        };
        assert_scales("chains", chains, 100, 4, 8);
    }
}
