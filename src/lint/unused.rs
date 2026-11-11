//! UnusedPrivateField, UnusedPrivateMethod and UnusedVariable (CodeNarc's `unused` rules).
//!
//! A name counts as used when it appears anywhere it could be read: as an identifier, or as
//! a word in any string (`"$x"`, `getProperty('x')`, `@MethodSource('x')`, a template the
//! grammar left as text). Comments don't count. That over-counts (another variable of the
//! same name elsewhere also counts), which only ever hides a report, never invents one.
//! Every occurrence is indexed once, so the rules stay linear in the file's size: a local's
//! uses are found by binary search over the positions of its name.

use super::basic::{annotations, has_modifier, is_method};
use super::{Violation, at, words};
use crate::text;
use std::collections::HashMap;
use tree_sitter::Node;

/// The node kinds `check` looks at: declarations and scopes.
pub(super) const KINDS: &[&str] = &[
    "field_declaration",
    "method_declaration",
    "constructor_declaration",
    "local_variable_declaration",
    "block",
    "closure",
    "switch_block",
    "lambda_expression",
];

/// The kinds of the nodes that can reach a member by a computed name.
pub(super) const DYNAMIC: &[&str] = &[
    "quoted_identifier",
    "method_invocation",
    "subscript_expression",
];

/// The kinds of the nodes that can hold a name: identifiers, and the text of strings.
pub(super) const LEAVES: &[&str] = &[
    "identifier",
    "type_identifier",
    "string_fragment",
    "string_literal",
    "gstring_dollar_interpolation",
];

/// Byte offsets of each `wanted` name wherever it appears as a word outside comments, in
/// order. A filter on (length, first byte) keeps most words from being hashed at all.
fn index(src: &str, leaves: &[Node], wanted: Vec<String>) -> HashMap<String, Vec<usize>> {
    let mut maybe = [0u64; 256];
    for w in wanted.iter().filter(|w| !w.is_empty()) {
        maybe[usize::from(w.as_bytes()[0])] |= 1 << w.len().min(63);
    }
    let might = |w: &str| maybe[usize::from(w.as_bytes()[0])] & (1 << w.len().min(63)) != 0;
    let mut idx: HashMap<String, Vec<usize>> =
        wanted.into_iter().map(|w| (w, Vec::new())).collect();
    for n in leaves {
        // the source is already valid UTF-8, so slice it instead of re-checking each name
        let Some(t) = src.get(n.byte_range()) else {
            continue;
        };
        if t.is_empty() {
            continue;
        }
        let k = n.kind();
        if k == "identifier" || k == "type_identifier" {
            // the whole name, which can hold `$` (`$foo`, `a$b`)
            if might(t)
                && let Some(ps) = idx.get_mut(t)
            {
                ps.push(n.start_byte());
            }
            if !t.contains('$') {
                continue;
            }
        } else if n.child_count() != 0 {
            // a string with parts: its fragments and names come on their own
            continue;
        }
        for w in words(t) {
            if might(w)
                && let Some(ps) = idx.get_mut(w)
            {
                ps.push(n.start_byte() + (w.as_ptr() as usize - t.as_ptr() as usize));
            }
        }
    }
    idx
}

/// Occurrences of `name` in `range`, other than at `except`.
fn uses(
    idx: &HashMap<String, Vec<usize>>,
    name: &str,
    range: std::ops::Range<usize>,
    except: usize,
) -> usize {
    idx.get(name).map_or(0, |ps| {
        let lo = ps.partition_point(|&p| p < range.start);
        let hi = ps.partition_point(|&p| p < range.end);
        ps[lo..hi].iter().filter(|&&p| p != except).count()
    })
}

/// Methods Java serialization, Groovy's MOP or a framework call by name.
const CALLED_BY_NAME: &[&str] = &[
    "methodMissing",
    "propertyMissing",
    "invokeMethod",
    "getProperty",
    "setProperty",
    "readObject",
    "writeObject",
    "readResolve",
    "writeReplace",
    "readObjectNoData",
    // Spock fixture methods, private or not
    "setup",
    "cleanup",
    "setupSpec",
    "cleanupSpec",
    // GPars actor lifecycle methods
    "afterStart",
    "afterStop",
    "onInterrupt",
    "onTimeout",
    "onException",
];

/// Calls that reach a member through a name computed at run time.
const DYNAMIC_CALLS: &[&str] = &[
    "getProperty",
    "setProperty",
    "invokeMethod",
    "hasProperty",
    "respondsTo",
    "getMetaProperty",
    "getAt",
];

/// A string with nothing interpolated.
fn plain_string(n: Node) -> bool {
    n.kind() == "string_literal" && !super::has_interpolation(n)
}

/// The literal starts of member names the file computes at run time: `groupBy_` for
/// `this."groupBy_${kind}"`, and `""` (any name) for `getProperty(name)`,
/// `invokeMethod(n, a)` or `this[name]` with a name that isn't a plain string. A private
/// member whose name starts with one of them may be used.
fn dynamic_prefixes(src: &str, nodes: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    for n in nodes {
        match n.kind() {
            "quoted_identifier" if super::has_interpolation(*n) => {
                let t = text(*n, src).trim_start_matches(['"', '\'', '/']);
                out.push(t[..t.find('$').unwrap_or(t.len())].to_string());
            }
            "method_invocation" => {
                // `"parse_$kind"(x)`
                if let Some(f) = n.child_by_field_name("function")
                    && f.kind() == "string_literal"
                    && super::has_interpolation(f)
                {
                    let t = text(f, src).trim_start_matches(['"', '\'', '/']);
                    out.push(t[..t.find('$').unwrap_or(t.len())].to_string());
                }
                let f = n
                    .child_by_field_name("function")
                    .map_or("", |f| text(f, src));
                let first = n
                    .child_by_field_name("arguments")
                    .and_then(|a| a.named_child(0));
                if DYNAMIC_CALLS.contains(&f.rsplit('.').next().unwrap_or(""))
                    && !first.is_some_and(plain_string)
                {
                    out.push(String::new());
                }
            }
            "subscript_expression"
                if n.child_by_field_name("object")
                    .is_some_and(|o| text(o, src) == "this")
                    && !n.child_by_field_name("index").is_some_and(plain_string) =>
            {
                out.push(String::new())
            }
            _ => {}
        }
    }
    out
}

/// Class annotations whose transforms read private fields (`includeFields`, JPA entities,
/// builders), as CodeNarc ignores `@Entity` classes.
fn reads_fields(src: &str, class: Node) -> bool {
    let mut c = class.walk();
    class.children(&mut c).any(|a| {
        a.kind() == "annotation" && {
            let t = text(a, src);
            t.contains("includeFields")
                || [
                    "Entity",
                    "Embeddable",
                    "MappedSuperclass",
                    "Document",
                    "Builder",
                ]
                .iter()
                .any(|n| {
                    a.child_by_field_name("name")
                        .is_some_and(|x| text(x, src).rsplit('.').next() == Some(n))
                })
        }
    })
}

/// `nodes`, `leaves` and `dynamic`: the nodes of the kinds in `KINDS`, `LEAVES` and `DYNAMIC`,
/// in preorder.
pub(super) fn check(
    src: &str,
    nodes: &[Node],
    leaves: &[Node],
    dynamic: &[Node],
    v: &mut Vec<Violation>,
) {
    // declarations, and the scope of each local: the innermost enclosing block, closure,
    // lambda or switch, found with a stack over the preorder (tree-sitter's `parent()` walks
    // down from the root, so climbing parents would be quadratic in nesting depth)
    let mut decls = Vec::new();
    let mut scopes: Vec<usize> = Vec::new();
    let mut scope_of = HashMap::new();
    for n in nodes.iter().copied() {
        while scopes.last().is_some_and(|&end| end <= n.start_byte()) {
            scopes.pop();
        }
        match n.kind() {
            "field_declaration" | "method_declaration" => decls.push(n),
            "constructor_declaration" if is_method(src, n) => decls.push(n),
            "local_variable_declaration" => {
                decls.push(n);
                // `None` at a script's top level
                scope_of.insert(n.id(), scopes.last().copied());
            }
            "block" | "closure" | "switch_block" | "lambda_expression" => scopes.push(n.end_byte()),
            _ => {}
        }
    }
    // an annotation (`@Field`, `@Delegate`, `@Autowired`, `@Shared`, `@Provides`) can mean
    // something reads it by reflection or a transform rewrites it
    decls.retain(|d| {
        (if d.kind() == "local_variable_declaration" {
            scope_of.get(&d.id()).is_some_and(Option::is_some)
        } else {
            has_modifier(*d, "private")
        }) && annotations(src, *d).is_empty()
    });
    if decls.is_empty() {
        return;
    }
    let idx = index(src, leaves, wanted(src, &decls));
    let all = 0..src.len();
    // only private members need it, so only worked out for a file that has one
    let prefixes = std::cell::OnceCell::new();
    let maybe_dynamic = |name: &str| {
        prefixes
            .get_or_init(|| dynamic_prefixes(src, dynamic))
            .iter()
            .any(|p: &String| name.starts_with(p.as_str()))
    };
    for d in decls {
        match d.kind() {
            "field_declaration" => {
                if !has_modifier(d, "private")
                    || d.parent()
                        .and_then(|b| b.parent())
                        .is_some_and(|c| reads_fields(src, c))
                {
                    continue;
                }
                for (name, at_node) in declarators(src, d) {
                    if !matches!(name, "serialVersionUID" | "serialPersistentFields")
                        && uses(&idx, name, all.clone(), at_node.start_byte()) == 0
                        && !maybe_dynamic(name)
                    {
                        v.push(at(
                            at_node,
                            "UnusedPrivateField",
                            format!("The private field {name} is never used; remove it, or use it"),
                        ));
                    }
                }
            }
            "method_declaration" | "constructor_declaration" => {
                let Some(n) = d
                    .child_by_field_name("name")
                    .filter(|n| n.kind() == "identifier")
                else {
                    continue;
                };
                let name = text(n, src);
                if !has_modifier(d, "private") || CALLED_BY_NAME.contains(&name) {
                    continue;
                }
                // `getFoo()` / `isFoo()` / `setFoo(x)` are also called as the property `foo`
                let property_uses = property(name).map_or(0, |p| {
                    let lower = lcfirst(p);
                    uses(&idx, &lower, all.clone(), usize::MAX)
                        + uses(&idx, p, all.clone(), usize::MAX)
                });
                if uses(&idx, name, all.clone(), n.start_byte()) + property_uses == 0
                    && !maybe_dynamic(name)
                {
                    v.push(at(
                        n,
                        "UnusedPrivateMethod",
                        format!("The private method {name} is never called; remove it, or call it"),
                    ));
                }
            }
            _ => {
                if let Some(&Some(end)) = scope_of.get(&d.id()) {
                    local(src, &idx, d, end, v)
                }
            }
        }
    }
}

/// The names the declarations could be used by: each name, and for `getX` / `isX` / `setX`
/// the property `x` and `X`.
fn wanted(src: &str, decls: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    for d in decls {
        match d.kind() {
            "field_declaration" | "local_variable_declaration" => {
                out.extend(declarators(src, *d).into_iter().map(|(n, _)| n.to_string()))
            }
            _ => {
                let name = d.child_by_field_name("name").map_or("", |n| text(n, src));
                out.push(name.to_string());
                if let Some(p) = property(name) {
                    out.push(lcfirst(p));
                    out.push(p.to_string());
                }
            }
        }
    }
    out
}

/// `Foo` for `getFoo`, `isFoo` and `setFoo`.
fn property(name: &str) -> Option<&str> {
    ["get", "is", "set"]
        .iter()
        .find_map(|p| name.strip_prefix(p))
        .filter(|r| r.starts_with(|c: char| c.is_ascii_uppercase()))
}

fn lcfirst(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_lowercase().chain(c).collect())
}

/// The names a field or local declaration declares, with the node of each name.
fn declarators<'a>(src: &'a str, d: Node<'a>) -> Vec<(&'a str, Node<'a>)> {
    if let Some(n) = d.child_by_field_name("name") {
        return vec![(text(n, src), n)];
    }
    let mut c = d.walk();
    d.named_children(&mut c)
        .filter(|x| x.kind() == "variable_declarator")
        .filter_map(|x| x.child_by_field_name("name"))
        .map(|n| (text(n, src), n))
        .collect()
}

/// UnusedVariable: a local whose name never appears after it in its scope (the enclosing
/// block, closure or switch, which ends at `end`). `_` is exempt; so are tuple declarations
/// (`def (a, b) = f()`), where an unused part is usually deliberate, and a script's
/// top-level variables, which snippets and `evaluate`d scripts leave for the reader.
fn local(
    src: &str,
    idx: &HashMap<String, Vec<usize>>,
    d: Node,
    end: usize,
    v: &mut Vec<Violation>,
) {
    for (name, n) in declarators(src, d) {
        if name == "_" {
            continue;
        }
        if uses(idx, name, n.end_byte()..end, usize::MAX) == 0 {
            v.push(at(
                n,
                "UnusedVariable",
                format!("The variable {name} is never used; remove it, or use it"),
            ));
        }
    }
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
    fn private_fields() {
        let src = "class A {\n  private int unused = 1\n  private int used\n  private static final long serialVersionUID = 1L\n  @Delegate private Map m\n  private String named\n  private String inString\n  int pub\n  private a, b\n  def f() { used + this.a + \"${inString}\" + new A(named: 1) }\n}\n@ToString(includeFields = true)\nclass B {\n  private int shown\n}\n";
        assert_eq!(rules(src, "UnusedPrivateField"), vec![2, 9]);
    }

    #[test]
    fn private_methods() {
        let src = "class A {\n  private void unused() { }\n  private void called() { }\n  private getThing() { 1 }\n  private boolean isOn() { true }\n  private void byPointer() { }\n  private void byName() { }\n  private void methodMissing(String n, a) { }\n  private static helper() { }\n  @PostConstruct private void init() { }\n  void unusedButPublic() { }\n  def f() {\n    called()\n    thing\n    def p = this.&byPointer\n    invokeMethod('byName', null)\n    p()\n  }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateMethod"), vec![2, 5, 9]);
    }

    #[test]
    fn dynamic_member_names() {
        // glu CustomDeltaColumnDefinition: `this."groupBy_${groupBy}"`
        let src = "class A {\n  private Closure groupBy_count = { 1 }\n  private int other\n  private void parse_a() { }\n  def f(k) { this.\"groupBy_${k}\"() + \"parse_$k\"() }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateField"), vec![3]);
        assert_eq!(rules(src, "UnusedPrivateMethod"), vec![] as Vec<usize>);
        // any name at all
        for call in ["getProperty(k)", "invokeMethod(k, null)", "this[k]"] {
            let src = format!("class A {{\n  private int x\n  def f(k) {{ {call} }}\n}}\n");
            assert_eq!(
                rules(&src, "UnusedPrivateField"),
                vec![] as Vec<usize>,
                "{call}"
            );
        }
        // a plain string is a plain use
        let src =
            "class A {\n  private int x\n  private int y\n  def f() { getProperty('x') }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateField"), vec![3]);
        // Spock and GPars call these by name
        let src = "class S extends Specification {\n  private setup() { }\n  private void afterStart() { }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateMethod"), vec![] as Vec<usize>);
    }

    #[test]
    fn local_variables() {
        let src = "def f() {\n  def a = 1\n  def b = 2\n  int c = b\n  def (d, e) = g()\n  def s = 1\n  println \"$s\"\n  def x = 1\n  def k = { x }\n  k()\n  def _ = 3\n  def t\n  // t is mentioned here only\n}\nswitch (y) {\n  case 1:\n    def q = 1\n    break\n  case 2:\n    use(q)\n}\n";
        assert_eq!(rules(src, "UnusedVariable"), vec![2, 4, 12]);
        // names with `$` (groovy ValidNameTest)
        assert_eq!(
            rules(
                "def f() {\n  def $foo$foo = 1\n  def a$b = 2\n  g($foo$foo + a$b)\n  def $c = 3\n}\n",
                "UnusedVariable"
            ),
            vec![5]
        );
        // a use before the declaration is another variable
        assert_eq!(
            rules("def f(x) {\n  g(x)\n  def y = 1\n}\n", "UnusedVariable"),
            vec![3]
        );
        // a script's top-level variables, and annotated ones (`@Field`), are left alone
        assert_eq!(
            rules(
                "def x = 1\n@Field def y = 1\ndef f() {\n  @SuppressWarnings('x') def z = 1\n}\n",
                "UnusedVariable"
            ),
            vec![] as Vec<usize>
        );
    }

    #[test]
    fn unused_symbols_are_linear_in_file_size() {
        // 4,000 locals in one chain of nested blocks, then 4,000 private fields and methods.
        // Climbing parents per local (tree-sitter walks down from the root for each) took
        // minutes on this; a scan of the file per declaration would be quadratic too.
        let mut src = String::from("class A {\n  def g() {\n");
        for i in 0..4_000 {
            src.push_str(&format!("    def v{i} = {i}\n    if (v{i}) {{\n"));
        }
        src.push_str(&"    }\n".repeat(4_000));
        src.push_str("  }\n");
        for i in 0..4_000 {
            src.push_str(&format!(
                "  private int f{i}\n  private void m{i}() {{ }}\n"
            ));
        }
        src.push_str("}\n");
        let t = std::time::Instant::now();
        let vs = lint(&src).expect("parses");
        let count = |r: &str| vs.iter().filter(|v| v.rule == r).count();
        assert_eq!(count("UnusedPrivateField"), 4_000);
        assert_eq!(count("UnusedPrivateMethod"), 4_000);
        assert_eq!(count("UnusedVariable"), 0);
        assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
    }
}
