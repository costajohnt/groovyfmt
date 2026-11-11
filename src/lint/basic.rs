//! CodeNarc's `basic` rules that the syntax tree decides alone, plus the method checks from
//! `naming`, `design` and `exceptions` that need no type information. Semantics follow each
//! rule's CodeNarc implementation and tests (`org.codenarc.rule.basic.*`); where groovyfmt
//! is narrower, the doc comment says why.

use super::{Violation, at, has_interpolation, is_empty_block, is_literal};
use crate::text;
use std::collections::HashSet;
use tree_sitter::Node;

/// The node kinds `check` looks at.
pub(super) const KINDS: &[&str] = &[
    "block",
    "closure",
    "switch_case",
    "switch_default",
    "source_file",
    "finally_clause",
    "switch_block",
    "static_initializer",
    "instance_initializer",
    "cast_expression",
    "ternary_expression",
    "elvis_expression",
    "assert_statement",
    "binary_expression",
    "spaceship_expression",
    "method_invocation",
    "class_declaration",
    "method_declaration",
    "constructor_declaration",
    "throw_statement",
    "catch_clause",
];

/// Ids of the fields read on every binary expression and call. By name, each lookup
/// searches the grammar's list of field names.
struct Fields {
    operator: u16,
    function: u16,
    field: u16,
}

fn fields(n: Node) -> &'static Fields {
    static FIELDS: std::sync::OnceLock<Fields> = std::sync::OnceLock::new();
    FIELDS.get_or_init(|| {
        let lang = n.language();
        let id = |name: &str| lang.field_id_for_name(name).map_or(0, |f| f.get());
        Fields {
            operator: id("operator"),
            function: id("function"),
            field: id("field"),
        }
    })
}

/// `nodes`: the nodes of the kinds in `KINDS`, in preorder.
pub(super) fn check(src: &str, nodes: &[Node], v: &mut Vec<Violation>) {
    let Some(&first) = nodes.first() else { return };
    let f = fields(first);
    let mut cursor = first.walk();
    for &n in nodes {
        match n.kind() {
            "block" | "closure" | "switch_case" | "switch_default" | "source_file" => {
                dead_code(n, &mut cursor, v)
            }
            "finally_clause" => finally(n, v),
            "switch_block" if n.named_child_count() == 0 => v.push(at(
                n.parent().unwrap_or(n),
                "EmptySwitchStatement",
                "The switch statement is empty; remove it or add the cases it should handle".into(),
            )),
            "static_initializer" if is_empty_block(n.child_by_field_name("body")) => v.push(at(
                n,
                "EmptyStaticInitializer",
                "The static initializer is empty and can be removed".into(),
            )),
            "instance_initializer" if is_empty_block(n.child_by_field_name("body")) => v.push(at(
                n,
                "EmptyInstanceInitializer",
                "The instance initializer is empty and can be removed".into(),
            )),
            "cast_expression" => duplicate_set_value(src, n, v),
            "ternary_expression" | "elvis_expression" => constant_ternary(src, n, v),
            "assert_statement" => {
                if n.named_child(0).is_some_and(|c| constant_truth(src, c)) {
                    v.push(at(
                        n,
                        "ConstantAssertExpression",
                        "The assert condition is a constant, so it always passes or always fails; assert what should hold".into(),
                    ));
                }
            }
            "binary_expression" => match n.child_by_field_id(f.operator).map_or("", |o| o.kind()) {
                "==" => {
                    two_constants(src, n, v);
                    broken_oddness(src, n, v);
                }
                "!=" | "<" | "<=" | ">" | ">=" => two_constants(src, n, v),
                "||" | "&&" => broken_null_check(src, n, v),
                _ => {}
            },
            "spaceship_expression" => two_constants(src, n, v),
            "method_invocation" => {
                // `x.removeAll(y)`, `x.equals(y)`, `x.compareTo(y)`
                let method = n
                    .child_by_field_id(f.function)
                    .filter(|x| x.kind() == "field_access")
                    .and_then(|x| x.child_by_field_id(f.field))
                    .map_or("", |x| text(x, src));
                match method {
                    "removeAll" => remove_all_on_self(src, n, v),
                    "equals" | "compareTo" => constant_equals(src, n, v),
                    _ => {}
                }
            }
            "class_declaration" => equals_and_hash_code(src, n, v),
            "method_declaration" => method(src, n, v),
            "constructor_declaration" if is_method(src, n) => method(src, n, v),
            "throw_statement" => missing_new(src, n, v),
            "catch_clause" => exception_not_thrown(src, n, v),
            _ => {}
        }
    }
}

/// Without wrapping parentheses.
fn unparen(mut n: Node) -> Node {
    while n.kind() == "parenthesized_expression"
        && let Some(c) = n.named_child(0)
    {
        n = c;
    }
    n
}

fn is_comment(n: Node) -> bool {
    n.kind().contains("comment")
}

/// DeadCode: a statement after `return`, `throw`, `break` or `continue` in the same block.
/// Declarations are not dead (a script's methods and classes come after its `return`), and
/// neither is a labeled statement, which another path can reach. A `break` right after a
/// `return` or `throw` (the switch-case habit) is redundant but harmless, so it is left
/// alone. So is code on the jump's own line: the grammar splits some one-line returns in
/// two (`return pipeline { ... }`, `return o instanceof T t && ...`).
/// `c` is any cursor, reused across blocks.
fn dead_code<'a>(n: Node<'a>, c: &mut tree_sitter::TreeCursor<'a>, v: &mut Vec<Violation>) {
    c.reset(n);
    if !c.goto_first_child() {
        return;
    }
    let mut ended: Option<Node> = None;
    loop {
        let s = c.node();
        let k = if s.is_named() { s.kind() } else { "" };
        if let Some(jump) = ended
            && !k.is_empty()
            && !k.ends_with("comment")
        {
            let declaration = k.ends_with("_declaration")
                && !matches!(
                    k,
                    "local_variable_declaration" | "multi_assignment_declaration"
                );
            let habit = k == "break_statement"
                && matches!(jump.kind(), "return_statement" | "throw_statement");
            if !declaration
                && !habit
                && k != "labeled_statement"
                && s.start_position().row > jump.end_position().row
            {
                v.push(at(
                    s,
                    "DeadCode",
                    "This code can never run: it follows a return, throw, break or continue; remove it or move it before".into(),
                ));
            }
            return;
        }
        if matches!(
            k,
            "return_statement" | "throw_statement" | "break_statement" | "continue_statement"
        ) {
            ended = Some(s);
        }
        if !c.goto_next_sibling() {
            return;
        }
    }
}

/// Whether `n` sits in the code of `top` itself, not in a closure, lambda, method or class
/// declared inside it, whose code runs some other time.
fn same_frame(n: Node, top: Node) -> bool {
    let mut p = n.parent();
    while let Some(a) = p {
        if a == top {
            return true;
        }
        if matches!(
            a.kind(),
            "closure" | "lambda_expression" | "class_body" | "method_declaration"
        ) {
            return false;
        }
        p = a.parent();
    }
    false
}

/// ReturnFromFinallyBlock, ThrowExceptionFromFinallyBlock, AssertWithinFinallyBlock. A
/// `throw` inside a `try` with a `catch`, within the `finally`, may be caught there, so it
/// is left alone.
fn finally(n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else {
        return;
    };
    crate::preorder(body, |x| {
        if x != body && matches!(x.kind(), "closure" | "lambda_expression" | "class_body") {
            return false;
        }
        match x.kind() {
            "return_statement" if same_frame(x, body) => v.push(at(
                x,
                "ReturnFromFinallyBlock",
                "Returning from a finally block discards the exception or return value of the try; move the return out of the finally".into(),
            )),
            "throw_statement" if same_frame(x, body) && !maybe_caught(x, body) => v.push(at(
                x,
                "ThrowExceptionFromFinallyBlock",
                "Throwing from a finally block hides the exception the try may have thrown; handle the error without throwing".into(),
            )),
            "assert_statement" if same_frame(x, body) => v.push(at(
                x,
                "AssertWithinFinallyBlock",
                "A failing assert in a finally block hides the exception the try may have thrown; check before the finally".into(),
            )),
            _ => {}
        }
        true
    });
}

/// `x` is in the body of a `try` with a `catch`, below `top`.
fn maybe_caught(x: Node, top: Node) -> bool {
    let mut p = x.parent();
    let mut child = x;
    while let Some(a) = p {
        if a == top {
            return false;
        }
        if a.kind() == "try_statement"
            && a.child_by_field_name("body") == Some(child)
            && a.children(&mut a.walk())
                .any(|c| c.kind() == "catch_clause")
        {
            return true;
        }
        child = a;
        p = a.parent();
    }
    false
}

/// The value of a constant element, for comparing set elements: `'a'` and `"a"` are equal,
/// `1` and `'1'` are not.
fn element(src: &str, n: Node) -> Option<(bool, String)> {
    if !is_literal(n) {
        return None;
    }
    super::constant_value(n, src).map(|s| (n.kind() == "string_literal", s))
}

/// DuplicateSetValue: `[1, 2, 1] as Set` (any type ending in `Set`).
fn duplicate_set_value(src: &str, n: Node, v: &mut Vec<Violation>) {
    let ty = n.child_by_field_name("type").map_or("", |t| text(t, src));
    let base = ty
        .split('<')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("");
    let Some(list) = n
        .child_by_field_name("value")
        .filter(|l| l.kind() == "list_literal" && base.ends_with("Set"))
    else {
        return;
    };
    let mut seen = HashSet::new();
    let mut c = list.walk();
    for e in list.named_children(&mut c) {
        if let Some(k) = element(src, e)
            && !seen.insert(k)
        {
            v.push(at(
                e,
                "DuplicateSetValue",
                format!(
                    "The value {} appears more than once in this {base}, which keeps only one; remove the copy or fix the value",
                    text(e, src)
                ),
            ));
        }
    }
}

/// A condition whose truth is fixed: a literal, `Boolean.TRUE` / `Boolean.FALSE`, or a list
/// or map literal (true unless empty) without a spread.
fn constant_truth(src: &str, n: Node) -> bool {
    let n = unparen(n);
    match n.kind() {
        "list_literal" | "map_literal" => {
            let mut c = n.walk();
            n.named_children(&mut c)
                .all(|e| !matches!(e.kind(), "spread_arguments" | "spread_map_entry"))
        }
        "field_access" => matches!(text(n, src), "Boolean.TRUE" | "Boolean.FALSE"),
        _ => is_literal(n),
    }
}

/// ConstantTernaryExpression: `true ? a : b`, `'x' ?: y`.
fn constant_ternary(src: &str, n: Node, v: &mut Vec<Violation>) {
    let field = if n.kind() == "ternary_expression" {
        "condition"
    } else {
        "value"
    };
    if n.child_by_field_name(field)
        .is_some_and(|c| constant_truth(src, c))
    {
        v.push(at(
            n,
            "ConstantTernaryExpression",
            "The condition of this ternary is a constant, so it always picks the same branch; use that branch directly".into(),
        ));
    }
}

/// A value made only of constants: literals, `Boolean.TRUE`, and list and map literals of them.
fn constant_value(src: &str, n: Node) -> bool {
    let mut ok = true;
    crate::preorder(unparen(n), |x| {
        ok &= match x.kind() {
            "list_literal" | "map_literal" | "map_entry" | "parenthesized_expression" => true,
            "number_literal" | "boolean_literal" | "null_literal" => true,
            "string_literal" => !has_interpolation(x),
            // a map key `a:` is a string
            "identifier" => x.parent().is_some_and(|p| {
                p.kind() == "map_entry" && p.child_by_field_name("key") == Some(x)
            }),
            "field_access" => matches!(text(x, src), "Boolean.TRUE" | "Boolean.FALSE"),
            _ => !x.is_named() || x.kind() == "string_fragment",
        };
        ok && x.kind() != "field_access" && x.kind() != "string_literal"
    });
    ok
}

/// ComparisonOfTwoConstants: `1 == 2`, `[a: 1] == [a: 2]`.
fn two_constants(src: &str, n: Node, v: &mut Vec<Violation>) {
    let op = n.child_by_field_name("operator").map_or("", |o| o.kind());
    if n.kind() == "binary_expression" && !matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=") {
        return;
    }
    if let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) && constant_value(src, l)
        && constant_value(src, r)
    {
        v.push(at(
            n,
            "ComparisonOfTwoConstants",
            format!(
                "Comparing two constants always gives the same answer: {}; compare the value that varies",
                text(n, src)
            ),
        ));
    }
}

/// ComparisonOfTwoConstants in its method forms: `[1, 2].equals([3])`, `'a'.compareTo('b')`.
fn constant_equals(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(f) = n
        .child_by_field_name("function")
        .filter(|f| f.kind() == "field_access")
    else {
        return;
    };
    if !f
        .child_by_field_name("field")
        .is_some_and(|x| matches!(text(x, src), "equals" | "compareTo"))
    {
        return;
    }
    let args = n
        .child_by_field_name("arguments")
        .filter(|a| a.kind() == "argument_list");
    if let (Some(o), Some(a)) = (f.child_by_field_name("object"), args)
        && a.named_child_count() == 1
        && constant_value(src, o)
        && a.named_child(0).is_some_and(|x| constant_value(src, x))
    {
        v.push(at(
            n,
            "ComparisonOfTwoConstants",
            format!(
                "Comparing two constants always gives the same answer: {}; compare the value that varies",
                text(n, src)
            ),
        ));
    }
}

/// Whether a `constructor_declaration` is really a method: the grammar reads a method with
/// modifiers and no return type (`private static helper() { }`) as a constructor unless its
/// name is the class's.
pub(super) fn is_method(src: &str, n: Node) -> bool {
    let class = n
        .parent()
        .and_then(|b| b.parent())
        .and_then(|c| c.child_by_field_name("name"));
    n.child_by_field_name("name")
        .is_some_and(|m| class.is_none_or(|c| text(c, src) != text(m, src)))
}

/// The name `x` in `x == null` / `null != x`, for `==` or `!=` as `op`.
fn null_tested<'a>(src: &'a str, n: Node, op: &str) -> Option<&'a str> {
    let n = unparen(n);
    if n.kind() != "binary_expression"
        || n.child_by_field_name("operator").map(|o| o.kind()) != Some(op)
    {
        return None;
    }
    let (l, r) = (
        n.child_by_field_name("left")?,
        n.child_by_field_name("right")?,
    );
    let x = match (l.kind(), r.kind()) {
        (_, "null_literal") => l,
        ("null_literal", _) => r,
        _ => return None,
    };
    is_path(x).then(|| text(x, src))
}

/// A variable or a chain of plain property reads, `a` or `this.a.b`: the same text means the
/// same value.
fn is_path(n: Node) -> bool {
    match n.kind() {
        "identifier" => true,
        "field_access" => {
            n.child_by_field_name("object").is_some_and(is_path)
                && n.child_by_field_name("field")
                    .is_some_and(|f| f.kind() == "identifier")
        }
        _ => false,
    }
}

/// The object a dereference reads, after any `!` and down the left of comparisons and
/// boolean operators: `x` in `x.size() > 0`, `!x.empty`, `x.a && y`.
fn dereferenced(n: Node) -> Option<Node> {
    let mut n = unparen(n);
    loop {
        n = match n.kind() {
            "unary_expression" => n.child_by_field_name("operand")?,
            "binary_expression" => n.child_by_field_name("left")?,
            "parenthesized_expression" => n.named_child(0)?,
            "method_invocation" => {
                let f = n.child_by_field_name("function")?;
                if f.kind() != "field_access" {
                    return None;
                }
                return f.child_by_field_name("object");
            }
            "field_access" | "subscript_expression" => return n.child_by_field_name("object"),
            _ => return None,
        };
    }
}

/// BrokenNullCheck: `x != null || x.a`, `x == null && x.a`, where the right side
/// dereferences exactly when `x` is null.
fn broken_null_check(src: &str, n: Node, v: &mut Vec<Violation>) {
    let op = n.child_by_field_name("operator").map_or("", |o| o.kind());
    let test = match op {
        "||" => "!=",
        "&&" => "==",
        _ => return,
    };
    let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) else {
        return;
    };
    if let Some(x) = null_tested(src, l, test)
        && dereferenced(r).is_some_and(|o| text(o, src) == x)
    {
        v.push(at(
            n,
            "BrokenNullCheck",
            format!(
                "`{x}` is dereferenced exactly when it is null, which throws a NullPointerException; did you mean {}?",
                if op == "||" { "&&" } else { "||" }
            ),
        ));
    }
}

/// BrokenOddnessCheck: `x % 2 == 1`, false for negative odd numbers.
fn broken_oddness(src: &str, n: Node, v: &mut Vec<Violation>) {
    if n.child_by_field_name("operator").map(|o| o.kind()) != Some("==") {
        return;
    }
    let is_mod2 = |m: Node| {
        let m = unparen(m);
        m.kind() == "binary_expression"
            && m.child_by_field_name("operator").map(|o| o.kind()) == Some("%")
            && m.child_by_field_name("right")
                .is_some_and(|r| text(r, src) == "2")
    };
    let is_one = |m: Node| text(unparen(m), src) == "1";
    if let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) && ((is_mod2(l) && is_one(r)) || (is_one(l) && is_mod2(r)))
    {
        v.push(at(
            n,
            "BrokenOddnessCheck",
            format!(
                "`{}` is false for negative odd numbers (-3 % 2 is -1); use `% 2 != 0`",
                text(n, src)
            ),
        ));
    }
}

/// RemoveAllOnSelf: `list.removeAll(list)`.
fn remove_all_on_self(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(f) = n
        .child_by_field_name("function")
        .filter(|f| f.kind() == "field_access")
    else {
        return;
    };
    if f.child_by_field_name("field").map(|x| text(x, src)) != Some("removeAll") {
        return;
    }
    let Some(o) = f.child_by_field_name("object").filter(|o| is_path(*o)) else {
        return;
    };
    let args = n.child_by_field_name("arguments");
    if let Some(a) = args
        && a.named_child_count() == 1
        && a.named_child(0)
            .is_some_and(|x| text(x, src) == text(o, src))
    {
        v.push(at(
            n,
            "RemoveAllOnSelf",
            format!(
                "`{}` empties the collection less clearly than `{}.clear()`, and can throw ConcurrentModificationException; use clear()",
                text(n, src),
                text(o, src)
            ),
        ));
    }
}

/// Whether `n` (a declaration) has the modifier token `m`.
pub(super) fn has_modifier(n: Node, m: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c).any(|k| k.kind() == m)
}

/// The simple names of the annotations on a declaration.
pub(super) fn annotations<'a>(src: &'a str, n: Node) -> Vec<&'a str> {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|a| a.kind() == "annotation")
        .filter_map(|a| a.child_by_field_name("name"))
        .map(|a| text(a, src).rsplit('.').next().unwrap_or(""))
        .collect()
}

/// A method's parameters.
fn params(m: Node) -> Vec<Node> {
    m.child_by_field_name("parameters")
        .map_or_else(Vec::new, |p| {
            let mut c = p.walk();
            p.named_children(&mut c)
                .filter(|x| x.kind() == "formal_parameter")
                .collect()
        })
}

/// A parameter declared `Object`, `java.lang.Object`, `def` or untyped.
fn object_param(src: &str, p: Node) -> bool {
    p.child_by_field_name("type")
        .is_none_or(|t| matches!(text(t, src), "Object" | "java.lang.Object" | "def"))
}

fn method_name<'a>(src: &'a str, m: Node) -> &'a str {
    m.child_by_field_name("name").map_or("", |n| text(n, src))
}

/// EqualsAndHashCode: a class that defines `equals(Object)` without `hashCode()`. Not when an
/// annotation generates it (`@EqualsAndHashCode`, `@Canonical`, `@Immutable`, `@Value`-style
/// transforms). CodeNarc also flags `hashCode()` without `equals`, but that keeps the
/// contract (identity equality, any hash), and on real code it only found deliberate test
/// fixtures, so groovyfmt doesn't.
fn equals_and_hash_code(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else {
        return;
    };
    let (mut equals, mut hash) = (false, false);
    let mut c = body.walk();
    for m in body.named_children(&mut c).filter(|m| {
        matches!(m.kind(), "method_declaration" | "constructor_declaration")
            && matches!(method_name(src, *m), "equals" | "hashCode")
            && !has_modifier(*m, "static")
    }) {
        let p = params(m);
        match method_name(src, m) {
            "equals" if p.len() == 1 && object_param(src, p[0]) => equals = true,
            "hashCode" if p.is_empty() => hash = true,
            _ => {}
        }
    }
    if !equals || hash {
        return;
    }
    let generated = annotations(src, n).iter().any(|a| {
        matches!(
            *a,
            "EqualsAndHashCode" | "Canonical" | "Immutable" | "Value" | "AutoValue" | "Data"
        )
    });
    // a superclass can supply the hashCode()
    let mut c = n.walk();
    if !generated && !n.children(&mut c).any(|k| k.kind() == "superclass") {
        let name = n.child_by_field_name("name").map_or("", |x| text(x, src));
        v.push(at(
            n.child_by_field_name("name").unwrap_or(n),
            "EqualsAndHashCode",
            format!(
                "Class {name} defines equals(Object) but not hashCode(); equal objects must have equal hash codes, so define hashCode() too"
            ),
        ));
    }
}

/// EqualsOverloaded, ObjectOverrideMisspelledMethodName, ToStringReturnsNull.
fn method(src: &str, m: Node, v: &mut Vec<Violation>) {
    let name = method_name(src, m);
    // only equal(s), hashCode and toString in any case
    if !matches!(name.len(), 5 | 6 | 8) || has_modifier(m, "static") {
        return;
    }
    let p = params(m);
    let at_name = |rule, msg: String| at(m.child_by_field_name("name").unwrap_or(m), rule, msg);
    // only the one-parameter form looks like an override; `equals(a, b)` implementing an
    // interface (Hibernate's UserType) or a Spock feature named `equals` is something else
    if name == "equals" && p.len() == 1 && !object_param(src, p[0]) {
        v.push(at_name(
            "EqualsOverloaded",
            "This equals() takes a typed parameter, so it overloads equals instead of overriding it, and == and collections ignore it; declare equals(Object)".into(),
        ));
    }
    let lower = name.to_ascii_lowercase();
    let misspelled = match lower.as_str() {
        "equals" | "equal" => name != "equals" && p.len() == 1 && object_param(src, p[0]),
        "hashcode" => name != "hashCode" && p.is_empty(),
        "tostring" => name != "toString" && p.is_empty(),
        _ => false,
    };
    if misspelled {
        let right = match lower.as_str() {
            "hashcode" => "hashCode",
            "tostring" => "toString",
            _ => "equals",
        };
        v.push(at_name(
            "ObjectOverrideMisspelledMethodName",
            format!("`{name}` looks like a misspelling of {right}, so it doesn't override Object.{right}; rename it to {right}"),
        ));
    }
    if name == "toString"
        && p.is_empty()
        && let Some(body) = m.child_by_field_name("body")
    {
        to_string_returns_null(body, v);
    }
}

/// ToStringReturnsNull: `return null` in toString(), a last statement `null`, or an empty
/// body (which returns null).
fn to_string_returns_null(body: Node, v: &mut Vec<Violation>) {
    let msg = || {
        "toString() returns null here, which breaks string concatenation and logging; return a string".to_string()
    };
    let mut c = body.walk();
    let stmts: Vec<Node> = body
        .named_children(&mut c)
        .filter(|s| !is_comment(*s))
        .collect();
    match stmts.last() {
        None => v.push(at(body, "ToStringReturnsNull", msg())),
        Some(s)
            if s.kind() == "expression_statement"
                && s.named_child(0)
                    .is_some_and(|e| unparen(e).kind() == "null_literal") =>
        {
            v.push(at(*s, "ToStringReturnsNull", msg()))
        }
        _ => {}
    }
    crate::preorder(body, |x| {
        if matches!(x.kind(), "closure" | "lambda_expression" | "class_body") {
            return false;
        }
        if x.kind() == "return_statement"
            && x.named_child(0)
                .is_some_and(|e| unparen(e).kind() == "null_literal")
        {
            v.push(at(x, "ToStringReturnsNull", msg()));
        }
        true
    });
}

/// A simple class name that reads as an exception: capitalized, ending in `Exception`,
/// `Failure` or `Fault`.
fn exception_like(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && ["Exception", "Failure", "Fault"]
            .iter()
            .any(|s| name.ends_with(s))
}

/// MissingNewInThrowStatement: `throw RuntimeException('x')`, which calls a method that
/// doesn't exist instead of creating the exception.
fn missing_new(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(e) = n.named_child(0).map(unparen) else {
        return;
    };
    // `throw RuntimeException('x')`, or `throw RuntimeException`, which throws a Class
    let f = if e.kind() == "method_invocation" {
        e.child_by_field_name("function")
    } else {
        Some(e)
    };
    if let Some(f) = f
        && f.kind() == "identifier"
        && exception_like(text(f, src))
    {
        v.push(at(
            e,
            "MissingNewInThrowStatement",
            format!(
                "This throw is missing `new`: it calls a method named {} (or throws the class itself) instead of creating the exception; write `throw new {0}(...)`",
                text(f, src)
            ),
        ));
    }
}

/// ExceptionNotThrown: a catch block ending in `new SomeException(...)` that isn't thrown.
/// Not when that value is what the method or closure returns (Groovy returns the last
/// expression), unless the method is `void`.
fn exception_not_thrown(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else {
        return;
    };
    let mut c = body.walk();
    let Some(last) = body
        .named_children(&mut c)
        .filter(|s| !is_comment(*s))
        .last()
    else {
        return;
    };
    let Some(e) = last.named_child(0).filter(|e| {
        last.kind() == "expression_statement" && e.kind() == "object_creation_expression"
    }) else {
        return;
    };
    let ty = e.child_by_field_name("type").map_or("", |t| text(t, src));
    let simple = ty
        .split('<')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("");
    if !simple.ends_with("Exception") || e.child_by_field_name("body").is_some() || returned(src, n)
    {
        return;
    }
    v.push(at(
        e,
        "ExceptionNotThrown",
        format!("This creates a {simple} and drops it; did you mean `throw new {simple}(...)`?"),
    ));
}

/// Whether the value of the catch clause `n` can be the value its method or closure returns:
/// the `try` is the last statement there, possibly through more blocks and branches.
fn returned(src: &str, n: Node) -> bool {
    let mut x = n.parent(); // the try
    while let Some(s) = x {
        let Some(p) = s.parent() else { return false };
        match p.kind() {
            "block" | "closure" => {
                let mut c = p.walk();
                let last = p.named_children(&mut c).filter(|k| !is_comment(*k)).last();
                if last != Some(s) {
                    return false;
                }
                let Some(owner) = p.parent().filter(|_| p.kind() == "block") else {
                    // a closure returns its last value
                    return true;
                };
                match owner.kind() {
                    "method_declaration" => {
                        return owner
                            .child_by_field_name("return_type")
                            .is_none_or(|t| text(t, src) != "void");
                    }
                    "constructor_declaration" | "static_initializer" | "instance_initializer" => {
                        return false;
                    }
                    "lambda_expression" => return true,
                    _ => x = Some(owner),
                }
            }
            "if_statement" | "labeled_statement" | "try_statement" | "catch_clause"
            | "finally_clause" | "switch_case" | "switch_default" | "switch_block" => x = Some(p),
            "source_file" => return true,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::lint::lint;

    fn rules(src: &str, rule: &str) -> Vec<usize> {
        lint(src)
            .expect("parses")
            .into_iter()
            .filter(|v| v.rule == rule)
            .map(|v| v.line)
            .collect()
    }

    #[test]
    fn dead_code() {
        let src = "def f() {\n  return 1\n  println 'x'\n}\ndef g(x) {\n  if (x) return 1\n  2\n}\nwhile (a) {\n  break\n  // why\n}\nreturn\ndef h() { }\nclass K { }\nswitch (x) {\n  case 1:\n    throw new E()\n    foo()\n}\nx = { -> return 1\n 2 }\n";
        assert_eq!(rules(src, "DeadCode"), vec![3, 19, 22]);
        // a labeled statement can be reached by a jump
        assert_eq!(
            rules("def f() {\n  return\n  a: f()\n}\n", "DeadCode"),
            vec![] as Vec<usize>
        );
        // `break` after a return in a switch is a habit; code on the jump's line is left to
        // the grammar, which splits `return pipeline { }` (orca DependentPipelineStarterSpec)
        let src = "switch (x) {\n  case 1:\n    return 1\n    break\n  case 2:\n    throw new E()\n    break\n}\nx = { _, p ->\n  return pipeline {\n    name = p\n  }\n}\nwhile (a) {\n  break\n  break\n}\n";
        assert_eq!(rules(src, "DeadCode"), vec![16]);
    }

    #[test]
    fn finally_blocks() {
        let src = "try { f() } finally {\n  return 1\n}\ntry { f() } finally {\n  throw new E()\n}\ntry { f() } finally {\n  assert x\n}\ntry { f() } finally {\n  xs.each { return it }\n  try { throw new E() } catch (e) { }\n  def g = { assert x }\n}\n";
        assert_eq!(rules(src, "ReturnFromFinallyBlock"), vec![2]);
        assert_eq!(rules(src, "ThrowExceptionFromFinallyBlock"), vec![5]);
        assert_eq!(rules(src, "AssertWithinFinallyBlock"), vec![8]);
    }

    #[test]
    fn empty_switch_and_initializers() {
        let src = "switch (x) { }\nswitch (x) {\n  // nothing yet\n}\nclass A {\n  static { }\n  { }\n  static { f() }\n  { g() }\n}\n";
        assert_eq!(rules(src, "EmptySwitchStatement"), vec![1]);
        assert_eq!(rules(src, "EmptyStaticInitializer"), vec![6]);
        assert_eq!(rules(src, "EmptyInstanceInitializer"), vec![7]);
    }

    #[test]
    fn set_values() {
        let src = "def a = [1, 2, 1] as Set\ndef b = ['a', \"a\"] as HashSet\ndef c = [1, '1', 1L, 1.0] as Set\ndef d = [1, 1] as List\ndef e = [x, x] as Set\ndef f = [\"${x}\", \"${x}\"] as Set\n";
        assert_eq!(rules(src, "DuplicateSetValue"), vec![1, 2]);
    }

    #[test]
    fn constant_conditions() {
        let src = "def a = true ? 1 : 2\ndef b = 'x' ?: 2\ndef c = [:] ? 1 : 2\ndef d = Boolean.TRUE ? 1 : 2\ndef e = x ? 1 : 2\ndef f = [*xs] ? 1 : 2\ndef g = \"${x}\" ?: 2\nassert true\nassert x\nassert null, 'm'\n";
        assert_eq!(rules(src, "ConstantTernaryExpression"), vec![1, 2, 3, 4]);
        assert_eq!(rules(src, "ConstantAssertExpression"), vec![8, 10]);
    }

    #[test]
    fn comparisons_of_constants() {
        let src = "if (1 == 2) { f() }\nx = 'a' < 'b'\ny = [1, [a: true]] != [2]\nz = x == 1\nw = [x] == [1]\nv = Boolean.FALSE != false\nu = \"${a}\" == 'a'\nt = [a: 1] <=> [a: 2]\ns = [1].equals([2])\nr = 'a'.compareTo('b')\nq = 'a'.equals(x)\n";
        assert_eq!(
            rules(src, "ComparisonOfTwoConstants"),
            vec![1, 2, 3, 6, 8, 9, 10]
        );
    }

    #[test]
    fn null_and_oddness_checks() {
        let src = "if (x != null || x.size() > 0) { f() }\nif (x == null && x.a) { f() }\nif (r == null && !r.id) { f() }\nif (x != null && x.size() > 0) { f() }\nif (x == null || x.a) { f() }\nif (x != null || y.a) { f() }\nif (x != null || x?.a) { f() }\nif (this.a != null || this.a.b) { f() }\n";
        assert_eq!(rules(src, "BrokenNullCheck"), vec![1, 2, 3, 8]);
        let src = "if (x % 2 == 1) { f() }\nif (1 == f() % 2) { f() }\nif (x % 2 != 0) { f() }\nif (x % 2 == 0) { f() }\nif (x % 3 == 1) { f() }\n";
        assert_eq!(rules(src, "BrokenOddnessCheck"), vec![1, 2]);
    }

    #[test]
    fn remove_all_on_self() {
        let src = "a.removeAll(a)\nthis.a.removeAll(this.a)\na.removeAll(b)\nf().removeAll(f())\n";
        assert_eq!(rules(src, "RemoveAllOnSelf"), vec![1, 2]);
    }

    #[test]
    fn equals_and_hash_code() {
        let src = "class A {\n  boolean equals(Object o) { true }\n}\nclass B {\n  int hashCode() { 1 }\n}\nclass C {\n  boolean equals(o) { true }\n  int hashCode() { 1 }\n}\n@EqualsAndHashCode\nclass D {\n  boolean equals(Object o) { true }\n}\nclass E {\n  boolean equals(String s) { true }\n  static int hashCode() { 1 }\n}\nclass F extends A {\n  boolean equals(Object o) { true }\n}\n";
        assert_eq!(rules(src, "EqualsAndHashCode"), vec![1]);
        assert_eq!(rules(src, "EqualsOverloaded"), vec![16]);
    }

    #[test]
    fn object_methods() {
        let src = "class A {\n  boolean equal(Object o) { true }\n  boolean equal(int o) { true }\n  boolean equaLS(o) { true }\n  int hashcode() { 1 }\n  int hashcode(int v) { 1 }\n  String toSTring() { '' }\n  String tostring(int v) { '' }\n  boolean equals(Object o, int x) { true }\n  boolean equals() { true }\n  static boolean equals(a, b) { true }\n}\n";
        assert_eq!(
            rules(src, "ObjectOverrideMisspelledMethodName"),
            vec![2, 4, 5, 7]
        );
        assert_eq!(rules(src, "EqualsOverloaded"), vec![] as Vec<usize>);
        // the grammar reads a method with modifiers and no return type as a constructor
        let src = "class A {\n  private tostring() { '' }\n  A() { }\n}\n";
        assert_eq!(rules(src, "ObjectOverrideMisspelledMethodName"), vec![2]);
    }

    #[test]
    fn to_string_returns_null() {
        let src = "class A {\n  String toString() {\n    if (x) {\n      return 'a'\n    }\n    return null\n  }\n}\nclass B {\n  String toString() {\n    f()\n    null\n  }\n}\nclass C {\n  String toString() { }\n}\nclass D {\n  String toString() { xs.find { return null } ?: 'd' }\n  String toString(int i) { null }\n}\n";
        assert_eq!(rules(src, "ToStringReturnsNull"), vec![6, 12, 16]);
    }

    #[test]
    fn throws() {
        let src = "throw RuntimeException('x')\nthrow new RuntimeException('x')\nthrow runtimeException()\nthrow Fault()\nthrow a.BadException()\nthrow IOException\nthrow e\n";
        assert_eq!(rules(src, "MissingNewInThrowStatement"), vec![1, 4, 6]);
        let src = "void f() {\n  try { g() } catch (e) { new IllegalStateException(e) }\n}\ndef h() {\n  try { g() } catch (e) {\n    new IllegalStateException(e)\n  }\n}\nvoid k() {\n  try { g() } catch (e) {\n    throw new IllegalStateException(e)\n  }\n  try { g() } catch (e) {\n    new IllegalStateException(e)\n  }\n  done()\n}\nx = { try { g() } catch (e) { new MyException() } }\n";
        assert_eq!(rules(src, "ExceptionNotThrown"), vec![2, 14]);
    }
}
