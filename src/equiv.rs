//! The safety checks behind every edit: formatting, wrapping, `--lines` and `--fix` keep
//! every token and the shape of the parse tree, outside the ranges an edit rewrites.

use std::ops::Range;
use tree_sitter::{Node, Tree};

/// A node's kind, field in its parent and depth, in document order (see `shape`).
pub type Shape = Vec<(u16, Option<u16>, usize)>;

/// Whether `n` lies within one of `skip`.
fn skipped(n: Node, skip: &[Range<usize>]) -> bool {
    skip.iter()
        .any(|r| r.start <= n.start_byte() && n.end_byte() <= r.end)
}

/// Leaf tokens outside `skip`, as (kind, text). With `trim_comments`, comments are compared
/// without trailing whitespace, since stripping it from a line comment is a legitimate
/// formatting change; any other leaf, string fragments included, must match exactly.
pub fn tokens<'a>(
    src: &'a str,
    tree: &Tree,
    skip: &[Range<usize>],
    trim_comments: bool,
) -> Vec<(u16, &'a str)> {
    let mut v = Vec::new();
    crate::preorder(tree.root_node(), |n| {
        if skipped(n, skip) {
            return false;
        }
        if n.child_count() == 0 {
            let t = crate::text(n, src);
            let trim = trim_comments && n.kind().contains("comment");
            v.push((n.kind_id(), if trim { t.trim_end() } else { t }));
        }
        true
    });
    v
}

/// Visit each node outside `skip` as (kind, field, depth) in document order, until `f`
/// returns false. The depths fix the tree's structure, so two trees with equal shapes have
/// the same nodes, nested the same way, in the same roles. Spacing can keep the tokens and
/// still change the parse (`a [1]`).
fn each(tree: &Tree, skip: &[Range<usize>], mut f: impl FnMut((u16, Option<u16>, usize)) -> bool) {
    let mut c = tree.walk();
    let mut depth = 0;
    loop {
        let n = c.node();
        let out = skipped(n, skip);
        if !out && !f((n.kind_id(), c.field_id().map(|id| id.get()), depth)) {
            return;
        }
        if !out && c.goto_first_child() {
            depth += 1;
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return;
            }
            depth -= 1;
        }
    }
}

/// Every node outside `skip`, named or not, as (kind, field, depth) in document order.
pub fn shape(tree: &Tree, skip: &[Range<usize>]) -> Shape {
    let mut v = Vec::new();
    each(tree, skip, |x| {
        v.push(x);
        true
    });
    v
}

/// Whether `t` parsed cleanly to `want`, a whole-tree `shape`, without building its own.
pub fn has_shape(t: &Tree, want: &[(u16, Option<u16>, usize)]) -> bool {
    let mut i = 0;
    let mut same = !t.root_node().has_error();
    if same {
        each(t, &[], |x| {
            same = want.get(i) == Some(&x);
            i += 1;
            same
        });
    }
    same && i == want.len()
}

/// Whether `out` (parsed as `out_tree`) is `src` (parsed as `tree`) with only whitespace
/// changed: it parses, and has the same tokens and, with `same_shape`, the same tree.
pub fn whitespace_only(
    (src, tree): (&str, &Tree),
    (out, out_tree): (&str, &Tree),
    same_shape: bool,
) -> bool {
    !out_tree.root_node().has_error()
        && tokens(src, tree, &[], true) == tokens(out, out_tree, &[], true)
        && (!same_shape || shape(tree, &[]) == shape(out_tree, &[]))
}
