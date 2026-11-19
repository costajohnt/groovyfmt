//! UnusedPrivateField, UnusedPrivateMethod and UnusedVariable (CodeNarc's `unused` rules).
//!
//! A name counts as used when it appears anywhere it could be read: as an identifier, or as
//! a word in any string (`"$x"`, `getProperty('x')`, `@MethodSource('x')`, a template the
//! grammar left as text). Comments don't count. That over-counts (another variable of the
//! same name elsewhere also counts), which only ever hides a report, never invents one.
//! Every occurrence is indexed once, so the rules stay linear in the file's size: a local's
//! uses are found by binary search over the positions of its name.

use super::basic::{annotations, has_modifier, is_method};
use super::{Rule, Violation, at, words};
use crate::text;
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

pub(super) const RULES: &[Rule] = &[
    // Unused code (src/lint/unused.rs)
    Rule {
        name: "UnusedPrivateField",
        default: true,
        description: "a private field whose name appears nowhere else in the file, nor in another file of the project that names its class (not one with an annotation, serialVersionUID, or in a class whose annotation reads fields)",
        why: "A private field nothing reads is dead state: left over from a refactoring, or a sign that the code meant to use it doesn't. A private member used only by a DSL script run with the class as its delegate (a `DelegatingScript`, a builder) looks unused here; mark it `@SuppressWarnings('UnusedPrivateField')` (or the class, for all of them), since removing it breaks the script. A method at the top level of a `.gradle` script is also its project's, which another build script can call (`rootProject.f()`), so one whose name another `.gradle` script of the project mentions is not reported.",
        bad: "class Client {\n    private int retries = 3\n    def get(url) { url.toURL().text }\n}",
        good: "class Client {\n    def get(url) { url.toURL().text }\n}",
        fixable: false,
    },
    Rule {
        name: "UnusedPrivateMethod",
        default: true,
        description: "a private method whose name appears nowhere else in the file (for getX/isX/setX, nor the property x), nor in another file of the project that names its class",
        why: "A private method nothing calls is dead code that still has to be read and maintained. A private member used only by a DSL script run with the class as its delegate (a `DelegatingScript`, a builder) looks unused here; mark it `@SuppressWarnings('UnusedPrivateMethod')` (or the class, for all of them), since removing it breaks the script.",
        bad: "class Client {\n    def get(url) { url.toURL().text }\n    private String trimmed(s) { s.trim() }\n}",
        good: "class Client {\n    def get(url) { url.toURL().text }\n}",
        fixable: false,
    },
    Rule {
        name: "UnusedVariable",
        default: true,
        description: "a local variable whose name never appears after its declaration in its block (not a script's top-level variable, `_`, `ignored`, Spock's `E e = thrown()`, an annotated one, or a `def (a, b)` tuple)",
        why: "A variable nothing reads is dead code, or the sign of a typo: the code meant to use it and uses something else.",
        bad: "def total(items) {\n    def count = items.size()\n    items.sum()\n}",
        good: "def total(items) {\n    items.sum()\n}",
        fixable: false,
    },
];

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

/// Groovy's operator methods and the tokens that call them: `this << x` calls a private
/// `leftShift`, `a + b` a `plus`. No tokens: called without one (`obj(x)`, `if (obj)`).
const OPERATOR_METHODS: &[(&str, &[&str])] = &[
    ("plus", &["+", "+="]),
    ("minus", &["-", "-="]),
    ("multiply", &["*", "*="]),
    ("div", &["/", "/="]),
    ("mod", &["%", "%="]),
    ("remainder", &["%", "%="]),
    ("power", &["**", "**="]),
    ("or", &["|", "|="]),
    ("and", &["&", "&="]),
    ("xor", &["^", "^="]),
    ("next", &["++"]),
    ("previous", &["--"]),
    ("getAt", &["[", "?["]),
    ("putAt", &["["]),
    ("leftShift", &["<<", "<<="]),
    ("rightShift", &[">>", ">>="]),
    ("rightShiftUnsigned", &[">>>", ">>>="]),
    ("isCase", &["in", "case"]),
    ("isNotCase", &["!in"]),
    ("asType", &["as"]),
    ("compareTo", &["<=>", "<", ">", "<=", ">="]),
    ("equals", &["==", "!="]),
    ("negative", &["-"]),
    ("positive", &["+"]),
    ("bitwiseNegate", &["~"]),
    ("call", &[]),
    ("asBoolean", &[]),
];

/// Whether `name` is an operator method whose operator appears in `class` (or that needs
/// none). Any use of the operator counts, on whatever operand: that only hides a report.
fn operator_used(name: &str, class: Node) -> bool {
    let Some((_, ops)) = OPERATOR_METHODS.iter().find(|(m, _)| *m == name) else {
        return false;
    };
    if ops.is_empty() {
        return true;
    }
    let mut c = class.walk();
    let mut stack = vec![class];
    while let Some(n) = stack.pop() {
        if !n.is_named() && ops.contains(&n.kind()) {
            return true;
        }
        stack.extend(n.children(&mut c));
    }
    false
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
    // the last statement of each method (not `void`), closure and lambda body: its value is
    // the one returned. Not for Spock's feature (`def "adds"()`) and fixture methods, whose
    // value nothing reads.
    let mut returned = HashSet::new();
    let mut parameters = HashSet::new();
    for n in nodes.iter().copied() {
        while scopes.last().is_some_and(|&end| end <= n.start_byte()) {
            scopes.pop();
        }
        let body = match n.kind() {
            "method_declaration" | "constructor_declaration"
                if (n.kind() == "method_declaration" || is_method(src, n))
                    && n.child_by_field_name("return_type")
                        .is_none_or(|t| text(t, src) != "void")
                    && n.child_by_field_name("name").is_some_and(|m| {
                        m.kind() == "identifier"
                            && !matches!(
                                text(m, src),
                                "setup" | "cleanup" | "setupSpec" | "cleanupSpec"
                            )
                    }) =>
            {
                n.child_by_field_name("body")
            }
            "closure" => Some(n),
            "lambda_expression" => n
                .child_by_field_name("body")
                .filter(|b| b.kind() == "block"),
            _ => None,
        };
        returned.extend(body.and_then(last_statement));
        if n.kind() == "closure" {
            parameters.extend(misparsed_parameters(n));
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
            scope_of.get(&d.id()).is_some_and(Option::is_some) && !parameters.contains(&d.id())
        } else {
            has_modifier(*d, "private")
        }) && annotations(src, *d)
            .iter()
            .all(|a| matches!(*a, "SuppressWarnings" | "Deprecated"))
            && !type_read_as_name(*d)
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
            .get_or_init(|| {
                // `metaClass` can add or reach any member by name
                if src.contains("metaClass") {
                    vec![String::new()]
                } else {
                    dynamic_prefixes(src, dynamic)
                }
            })
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
                        v.push(Violation {
                            owner: owner(src, d),
                            ..at(
                                at_node,
                                "UnusedPrivateField",
                                format!(
                                    "The private field {name} is never used; remove it, or use it"
                                ),
                            )
                        });
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
                if !has_modifier(d, "private")
                    || CALLED_BY_NAME.contains(&name)
                    || d.parent().is_some_and(|body| operator_used(name, body))
                {
                    continue;
                }
                // `getFoo()` / `isFoo()` / `setFoo(x)` are also called as the property `foo`
                let property_uses: usize = property_names(name)
                    .iter()
                    .map(|p| uses(&idx, p, all.clone(), usize::MAX))
                    .sum();
                if uses(&idx, name, all.clone(), n.start_byte()) + property_uses == 0
                    && !maybe_dynamic(name)
                {
                    v.push(Violation {
                        owner: owner(src, d),
                        ..at(
                            n,
                            "UnusedPrivateMethod",
                            format!(
                                "The private method {name} is never called; remove it, or call it"
                            ),
                        )
                    });
                }
            }
            _ => {
                if let Some(&Some(end)) = scope_of.get(&d.id()) {
                    local(src, &idx, d, end, returned.contains(&d.id()), v)
                }
            }
        }
    }
}

/// The name of the class, trait, enum, interface or record declaring `d`.
fn owner(src: &str, d: Node) -> Option<String> {
    let mut p = d.parent();
    while let Some(x) = p {
        if matches!(
            x.kind(),
            "class_declaration"
                | "trait_declaration"
                | "enum_declaration"
                | "interface_declaration"
                | "record_declaration"
        ) {
            return x
                .child_by_field_name("name")
                .map(|n| text(n, src).to_string());
        }
        p = x.parent();
    }
    None
}

/// `final ClassNode name` with no value: the grammar reads `ClassNode` as the declared
/// name and `name` as a statement of its own on the same line (grails-core `AstUtils`), so
/// the name this declaration has is not the variable's. Left alone.
fn type_read_as_name(d: Node) -> bool {
    let mut c = d.walk();
    let no_value = d
        .named_children(&mut c)
        .filter(|x| x.kind() == "variable_declarator")
        .all(|x| x.child_by_field_name("value").is_none())
        && d.child_by_field_name("value").is_none();
    no_value
        && d.next_named_sibling().is_some_and(|s| {
            s.kind() == "expression_statement"
                && s.start_position().row == d.end_position().row
                && s.named_child(0).is_some_and(|e| e.kind() == "identifier")
                && s.named_child_count() == 1
        })
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
                out.extend(property_names(name));
            }
        }
    }
    out
}

/// `Foo` for `getFoo`, `isFoo` and `setFoo`. Any rest counts, as Groovy also reads
/// `get_foo` as `_foo`, `getaB` as `aB` and `getÄpfel` as `äpfel` (#181).
fn property(name: &str) -> Option<&str> {
    ["get", "is", "set"]
        .iter()
        .find_map(|p| name.strip_prefix(p))
        .filter(|r| !r.is_empty())
}

/// The names accessor `name` is read under as a property: `foo` and `Foo` for `getFoo`,
/// `SCOPES` for `getSCOPES` (Groovy leaves a name whose first two letters are capitals as
/// it is, #181). Groovy accepts only some of them for a given name; extras just keep it.
pub(crate) fn property_names(name: &str) -> Vec<String> {
    property(name).map_or_else(Vec::new, |p| vec![lcfirst(p), p.to_string()])
}

pub(super) fn lcfirst(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_lowercase().chain(c).collect())
}

/// The names a field or local declaration declares, with the node of each name.
pub(super) fn declarators<'a>(src: &'a str, d: Node<'a>) -> Vec<(&'a str, Node<'a>)> {
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
/// block, closure or switch, which ends at `end`). `_`, `ignore` and `ignored` are exempt, as
/// is Spock's `E e = thrown()`; so are tuple declarations
/// (`def (a, b) = f()`), where an unused part is usually deliberate, and a script's
/// top-level variables, which snippets and `evaluate`d scripts leave for the reader.
/// `returned`: `d` is the last statement of a method or closure, so the value of its last
/// declarator is the return value (`Foo getFoo() { Foo foo = lookup() }`), and removing it
/// would change what is returned.
fn local(
    src: &str,
    idx: &HashMap<String, Vec<usize>>,
    d: Node,
    end: usize,
    returned: bool,
    v: &mut Vec<Violation>,
) {
    let names = declarators(src, d);
    let count = names.len();
    for (i, (name, n)) in names.into_iter().enumerate() {
        // `ignored` says it on purpose, as for EmptyCatchBlock
        if matches!(name, "_" | "ignore" | "ignored") {
            continue;
        }
        if returned
            && i + 1 == count
            && n.parent()
                .is_some_and(|p| p.child_by_field_name("value").is_some())
        {
            continue;
        }
        if uses(idx, name, n.end_byte()..end, usize::MAX) == 0 && !spock_thrown(src, n) {
            v.push(at(
                n,
                "UnusedVariable",
                format!("The variable {name} is never used; remove it, or use it"),
            ));
        }
    }
}

/// `{ List<String> a, String b -> }` parses as the local `List<String> a, String` and the
/// lambda `b -> ...` (#182): the id of that local, which holds `closure`'s parameters.
fn misparsed_parameters(closure: Node) -> Option<usize> {
    let d = closure.named_child(0)?;
    (d.kind() == "local_variable_declaration"
        && d.next_named_sibling().is_some_and(|s| {
            s.kind() == "expression_statement"
                && s.named_child(0)
                    .is_some_and(|l| l.kind() == "lambda_expression")
        }))
    .then(|| d.id())
}

/// The id of the last statement of a body, comments aside.
fn last_statement(body: Node) -> Option<usize> {
    let mut c = body.walk();
    body.named_children(&mut c)
        .filter(|s| !s.is_extra())
        .last()
        .map(|s| s.id())
}

/// Spock's `IOException e = thrown()`: the declared type is the exception expected, so the
/// variable is there for its type.
fn spock_thrown(src: &str, name: Node) -> bool {
    name.parent()
        .and_then(|p| p.child_by_field_name("value"))
        .is_some_and(|v| {
            v.kind() == "method_invocation"
                && v.child_by_field_name("function")
                    .is_some_and(|f| text(f, src) == "thrown")
                && v.child_by_field_name("arguments")
                    .is_none_or(|a| a.named_child_count() == 0)
        })
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
    fn closure_parameters_with_a_generic_type_are_not_locals() {
        // #182: `{ List<String> ctx, String m -> }` parses as a local and a lambda
        let src = "def f() {\n  def c = { List<String> ctx, String m -> m }\n  def d = { Map<String, Integer> ctx, String m, int k -> m }\n  def e = { List<String> ctx, m -> m }\n  [c, d, e]\n}\n";
        assert_eq!(rules(src, "UnusedVariable"), vec![] as Vec<usize>);
        // a real first local in a closure is still checked
        let src = "def f() {\n  def c = {\n    List<String> unused = []\n    go()\n  }\n  c\n}\n";
        assert_eq!(rules(src, "UnusedVariable"), vec![3]);
    }

    #[test]
    fn private_methods() {
        let src = "class A {\n  private void unused() { }\n  private void called() { }\n  private getThing() { 1 }\n  private boolean isOn() { true }\n  private void byPointer() { }\n  private void byName() { }\n  private void methodMissing(String n, a) { }\n  private static helper() { }\n  @PostConstruct private void init() { }\n  void unusedButPublic() { }\n  def f() {\n    called()\n    thing\n    def p = this.&byPointer\n    invokeMethod('byName', null)\n    p()\n  }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateMethod"), vec![2, 5, 9]);
    }

    #[test]
    fn operator_methods_are_used_through_their_operator() {
        // #256: mircomp FactorOracle, `this << item`
        let src = "class A {\n  void add(x) { this << x }\n  private void leftShift(x) { }\n  private A plus(A o) { this }\n  private A minus(A o) { this }\n  def f(A a) { a + a }\n  private void call(x) { }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateMethod"), vec![5]);
        // the operator in another class doesn't count
        let src =
            "class A {\n  private void leftShift(x) { }\n}\nclass B {\n  def f(a) { a << 1 }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateMethod"), vec![2]);
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
    fn a_type_the_grammar_reads_as_the_name_is_never_reported() {
        // grails-core AstUtils: `final ClassNode placeHolderType`, assigned in each branch
        let src = "def f(t) {\n  final ClassNode placeHolderType\n  if (t) {\n    placeHolderType = g()\n  } else {\n    placeHolderType = h()\n  }\n  use(placeHolderType)\n}\n";
        assert_eq!(rules(src, "UnusedVariable"), vec![] as Vec<usize>);
        // a value-less local that parses as itself is still judged
        assert_eq!(
            rules("def f() {\n  final x\n}\n", "UnusedVariable"),
            vec![2]
        );
        // metaClass reaches members by name
        let src = "class A {\n  private int x\n  def f() { this.metaClass.y = { -> 1 } }\n}\n";
        assert_eq!(rules(src, "UnusedPrivateField"), vec![] as Vec<usize>);
    }

    #[test]
    fn local_variables() {
        let src = "def f() {\n  def a = 1\n  def b = 2\n  int c = b\n  def (d, e) = g()\n  def s = 1\n  println \"$s\"\n  def x = 1\n  def k = { x }\n  k()\n  def _ = 3\n  def t\n  // t is mentioned here only\n}\nswitch (y) {\n  case 1:\n    def q = 1\n    break\n  case 2:\n    use(q)\n}\n";
        assert_eq!(rules(src, "UnusedVariable"), vec![2, 4, 12]);
        // names with `$` (groovy ValidNameTest)
        assert_eq!(
            rules(
                "def f() {\n  def $foo$foo = 1\n  def a$b = 2\n  g($foo$foo + a$b)\n  def $c = 3\n  g()\n}\n",
                "UnusedVariable"
            ),
            vec![5]
        );
        // Spock's `E e = thrown()` is there for its type; `thrown(E)` and other calls are not,
        // and `ignored` is unused on purpose
        assert_eq!(
            rules(
                "def f() {\n  IOException e = thrown()\n  def x = thrown(IOException)\n  def y = other()\n  def ignored = g()\n}\n",
                "UnusedVariable"
            ),
            vec![3, 4]
        );
        // a use before the declaration is another variable
        assert_eq!(
            rules(
                "def f(x) {\n  g(x)\n  def y = 1\n  g()\n}\n",
                "UnusedVariable"
            ),
            vec![3]
        );
        // a script's top-level variables and annotated ones (`@Field`) are left alone, and
        // a suppression is a suppression
        assert_eq!(
            rules(
                "def x = 1\n@Field def y = 1\ndef f() {\n  @SuppressWarnings('UnusedVariable') def z = 1\n  g()\n}\n",
                "UnusedVariable"
            ),
            vec![] as Vec<usize>
        );
    }

    #[test]
    fn a_declaration_that_is_the_return_value_is_not_reported() {
        // the last statement of a method that returns a value, or of a closure, is returned
        let src = "class A {\n  private Foo getFoo() {\n    Foo foo = lookup()\n  }\n  def f() {\n    def x = 1\n  }\n  int g() {\n    int d = 3 // the result\n  }\n  void v() {\n    String a = 'x'\n  }\n  def h() {\n    def y = 1\n    g()\n  }\n  def k() {\n    def p = 1, q = 2\n  }\n  A() {\n    def z = 1\n  }\n}\ndef c = { ->\n  def b = 2\n}\ndef l = { x ->\n  def e = x\n  e + 1\n  def n = 1\n}\n";
        // `void` methods, a constructor, a statement after it, the other names of the
        // declaration
        assert_eq!(rules(src, "UnusedVariable"), vec![12, 15, 19, 22]);
        // Spock features and fixtures return nothing anyone reads
        let src = "class S extends Specification {\n  def setup() {\n    def a = 1\n  }\n  def \"adds\"() {\n    expect:\n    true\n    def b = 1\n  }\n}\n";
        assert_eq!(rules(src, "UnusedVariable"), vec![3, 8]);
    }

    #[test]
    fn unused_symbols_are_linear_in_file_size() {
        // n locals in one chain of nested blocks, then n private fields and methods. Climbing
        // parents per local (tree-sitter walks down from the root for each) took minutes on
        // 4,000; a scan of the file per declaration would be quadratic too.
        let run = |n: usize| {
            let mut src = String::from("class A {\n  def g() {\n");
            for i in 0..n {
                src.push_str(&format!("    def v{i} = {i}\n    if (v{i}) {{\n"));
            }
            src.push_str(&"    }\n".repeat(n));
            src.push_str("  }\n");
            for i in 0..n {
                src.push_str(&format!(
                    "  private int f{i}\n  private void m{i}() {{ }}\n"
                ));
            }
            src.push_str("}\n");
            let vs = lint(&src).expect("parses");
            let count = |r: &str| vs.iter().filter(|v| v.rule == r).count();
            assert_eq!(count("UnusedPrivateField"), n);
            assert_eq!(count("UnusedPrivateMethod"), n);
            assert_eq!(count("UnusedVariable"), 0);
        };
        // n blocks deep: no deeper than four times what was measured to take minutes
        crate::scaling::assert_scales_up_to("unused symbols", run, 1_000, 16_000, 4, 8);
    }
}
