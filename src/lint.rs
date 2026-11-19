//! Native lint rules, named after their CodeNarc equivalents so CI configs map across.
//! Files with parse errors are not linted (a broken tree would produce false positives).

use std::path::Path;
use tree_sitter::Node;

#[derive(Clone, Copy)]
pub struct Rule {
    pub name: &'static str,
    pub default: bool,
    pub description: &'static str,
    /// Why it matters, for `--explain`.
    pub why: &'static str,
    /// Code it flags, and the same code without the problem.
    pub bad: &'static str,
    pub good: &'static str,
    /// Whether `--lint --fix` fixes it.
    pub fixable: bool,
}

/// Rules with `fixable: false` whose reports `--lint --fix` fixes only in some cases.
pub const PARTLY_FIXABLE: &[&str] = &["GradleRemovedApi"];

/// Every rule, named after its CodeNarc equivalent so existing configs map across: each
/// module's own, in this order, which shows in `--rules`, docs/lint.md and SARIF's rule
/// indices.
pub const RULES: &[Rule] = &concat::<{ count(PARTS) }>(PARTS);

const PARTS: &[&[Rule]] = &[
    general::RULES,
    suppress::RULES,
    imports::RULES,
    basic::RULES,
    unused::RULES,
    jenkins::RULES,
    gradle::RULES,
    spock::RULES,
    basic::LATE_RULES,
];

const fn count(parts: &[&[Rule]]) -> usize {
    let (mut n, mut i) = (0, 0);
    while i < parts.len() {
        n += parts[i].len();
        i += 1;
    }
    n
}

const fn concat<const N: usize>(parts: &[&[Rule]]) -> [Rule; N] {
    let mut out = [parts[0][0]; N];
    let (mut k, mut i) = (0, 0);
    while i < parts.len() {
        let mut j = 0;
        while j < parts[i].len() {
            out[k] = parts[i][j];
            (k, j) = (k + 1, j + 1);
        }
        i += 1;
    }
    out
}

mod basic;
mod elsewhere;
mod general;
mod gradle;
mod imports;
mod jenkins;
mod spock;
mod suppress;
mod unused;

pub use elsewhere::{Context, Projects};
use general::{
    constant_value, has_interpolation, import_has, is_empty_block, is_literal, rebound, words,
};
pub(crate) use general::{
    dollar_names, dotted, hubitat_include, import_name, import_property_names, joins_declaration,
};
pub(crate) use gradle::{same_assignment, same_call};

#[derive(Debug, PartialEq)]
pub struct Violation {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based, in characters
    /// Where the flagged node ends: the line, and the column just after its last character.
    pub end_line: usize,
    pub end_col: usize,
    pub rule: &'static str,
    pub message: String,
    /// What `--fix` does to fix it, for the rules it fixes: delete the `;`, or the import's
    /// whole lines when nothing else is on them; replace a removed Gradle configuration's
    /// name; rewrite a Gradle statement.
    pub fix: Option<Edit>,
    /// The bytes of the flagged code, so `--fix` can tell a report that goes with a deleted
    /// line (an unused import that is also unnecessary) from one that changed.
    pub span: std::ops::Range<usize>,
    /// UnusedPrivateField and UnusedPrivateMethod: the type declaring the member (`None`
    /// at a script's top level), which another file must name to use it
    pub owner: Option<String>,
}

/// `--fix`'s change for one violation: the bytes `range` become `with` (empty: deleted).
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub range: std::ops::Range<usize>,
    pub with: String,
}

pub fn lint(src: &str) -> Result<Vec<Violation>, &'static str> {
    lint_file(Path::new(""), src)
}

/// Fixes that depend on the config: with `gradle_compile_replacement` set (`compile`),
/// GradleRemovedConfiguration on a `compile` dependency replaces the name with it.
pub fn opt_in_fixes(vs: &mut [Violation], src: &str, compile: Option<&str>) {
    if let Some(with) = compile {
        gradle::opt_in_compile(vs, src, with);
    }
}

/// `lint_with`, then `opt_in_fixes`: every violation, enabled or not, judged without the
/// project's Spock base classes. What `fix` checks its edits against; `run` adds the
/// config's and the project's filters.
pub fn lint_opt_in(
    path: &Path,
    src: &str,
    compile: Option<&str>,
    base: &Context,
) -> Result<Vec<Violation>, &'static str> {
    let mut vs = lint_with(path, src, base)?;
    opt_in_fixes(&mut vs, src, compile);
    Ok(vs)
}

/// What `src`, the text of the file at `path`, reports under `cfg`: the one pipeline for
/// `--lint`, `--lint --fix`, the language server's diagnostics and its code actions. Its
/// violations (`lint_opt_in`, Spock bases from the project) of the rules `cfg` enables, less
/// the unused private members another file of the project uses. `real` is where the file is
/// (a named link's target), which decides its project. The baseline is the caller's.
pub fn run(
    path: &Path,
    real: &Path,
    src: &str,
    cfg: &crate::config::Config,
    projects: &Projects,
) -> Result<Vec<Violation>, &'static str> {
    let compile = cfg.gradle_compile_replacement.as_deref();
    let mut vs = lint_opt_in(path, src, compile, &projects.spec_base(real))?;
    vs.retain(|v| cfg.rule_enabled(v.rule));
    projects.filter(real, src, &mut vs);
    Ok(vs)
}

/// Lint `src` as the file at `path`, whose name decides the Jenkinsfile and Gradle rules.
pub fn lint_file(path: &Path, src: &str) -> Result<Vec<Violation>, &'static str> {
    lint_with(path, src, &Context::new(|_| None))
}

/// `lint_file`, with `base` telling whether a superclass declared elsewhere is a Spock spec
/// and caching the Gradle files around a build script (`Projects::spec_base`).
pub fn lint_with(path: &Path, src: &str, base: &Context) -> Result<Vec<Violation>, &'static str> {
    let tree = crate::parse(src);
    if tree.root_node().has_error() {
        return Err("parse error");
    }
    let mut v = Vec::new();
    // this file's classes, for the Spock rules, read the first time a class needs them
    let local = std::cell::OnceCell::new();
    general::imports(src, &tree, &mut v);
    jenkins::check(path, src, tree.root_node(), &mut v);
    gradle::check(path, src, tree.root_node(), base.gradle, &mut v);
    let mut nodes = Vec::new();
    crate::preorder(tree.root_node(), |n| {
        nodes.push(n);
        true
    });
    // the nodes each of basic, unused and imports looks at, in preorder
    let table = kinds(&tree.language());
    let mut by_module: [Vec<Node>; 6] = Default::default();
    for (i, &n) in nodes.iter().enumerate() {
        let (kind, m) = table
            .get(usize::from(n.kind_id()))
            .copied()
            .unwrap_or(("", 0));
        if m != 0 {
            for (j, list) in by_module.iter_mut().enumerate() {
                if m & (1 << j) != 0 {
                    list.push(n);
                }
            }
        }
        general::node(src, kind, n, nodes.get(i + 1..).unwrap_or(&[]), &mut v);
        if kind == "class_declaration" {
            spock::class(src, n, &local, &*base.spec_base, &mut v);
        }
    }
    let [
        basic_nodes,
        unused_nodes,
        import_nodes,
        name_leaves,
        dynamic,
        directives,
    ] = by_module;
    basic::check(src, &basic_nodes, &mut v);
    basic::class_name_same_as_filename(path, src, tree.root_node(), &mut v);
    unused::check(src, &unused_nodes, &name_leaves, &dynamic, &mut v);
    imports::check(path, src, &import_nodes, &mut v);
    suppress::apply(src, &directives, &mut v);
    // tree-sitter columns count bytes; report characters, as editors and GitHub do
    let lines: Vec<&str> = src.split('\n').collect();
    let chars = |line: usize, col: usize| {
        lines
            .get(line - 1)
            .and_then(|l| l.get(..col - 1))
            .map_or(col, |prefix| crate::chars_before(prefix, line - 1) + 1)
    };
    for x in &mut v {
        x.col = chars(x.line, x.col);
        x.end_col = chars(x.end_line, x.end_col);
    }
    v.sort_by_key(|x| (x.line, x.col));
    Ok(v)
}

/// By node kind id: the kind's name, and a bit for each list in [basic, unused, imports,
/// unused's name leaves, unused's dynamic member access, suppressions] whose kinds include it. Looking the
/// name up here costs one call into tree-sitter per node instead of two, and each module
/// sees only its own nodes: a pass over every node per module cost 25% of `--lint`.
fn kinds(lang: &tree_sitter::Language) -> &'static [(&'static str, u8)] {
    static TABLE: std::sync::OnceLock<Vec<(&'static str, u8)>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let modules = [
            basic::KINDS,
            unused::KINDS,
            imports::KINDS,
            unused::LEAVES,
            unused::DYNAMIC,
            suppress::KINDS,
        ];
        (0..lang.node_kind_count())
            .map(|id| {
                let id = u16::try_from(id).unwrap_or(u16::MAX);
                let kind = lang.node_kind_for_id(id).unwrap_or("");
                let named = lang.node_kind_is_named(id);
                let bits = modules
                    .iter()
                    .enumerate()
                    .filter(|(_, ks)| named && ks.contains(&kind))
                    .fold(0, |m, (i, _)| m | 1 << i);
                (kind, bits)
            })
            .collect()
    })
}

fn at(n: Node, rule: &'static str, message: String) -> Violation {
    let (p, e) = (n.start_position(), n.end_position());
    Violation {
        line: p.row + 1,
        col: p.column + 1,
        end_line: e.row + 1,
        end_col: e.column + 1,
        rule,
        message,
        fix: None,
        span: n.byte_range(),
        owner: None,
    }
}

/// The lines holding `n`, with the line break after them, when nothing but whitespace and
/// a `;` shares them.
fn own_lines(src: &str, n: Node) -> Option<Edit> {
    // a UTF-8 BOM stays, and isn't something sharing the line
    let bom = if src.starts_with('\u{feff}') { 3 } else { 0 };
    let start = src[..n.start_byte()]
        .rfind('\n')
        .map_or(bom.min(n.start_byte()), |i| i + 1);
    let end = src[n.end_byte()..]
        .find('\n')
        .map_or(src.len(), |i| n.end_byte() + i + 1);
    let after = src[n.end_byte()..end].trim();
    (src[start..n.start_byte()].trim().is_empty() && (after.is_empty() || after == ";")).then(
        || Edit {
            range: start..end,
            with: String::new(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn rules(src: &str) -> Vec<(&'static str, usize)> {
        lint(src)
            .expect("parses")
            .into_iter()
            .map(|v| (v.rule, v.line))
            .collect()
    }

    #[test]
    fn imports_unused_duplicate_and_used() {
        let src = "import a.b.Used\nimport a.b.Unused\nimport a.b.Used\nimport a.c.*\nimport a.b.Linked\n\n/** see {@link Linked} */\nclass X { Used u }\n";
        assert_eq!(
            rules(src),
            vec![("UnusedImport", 2), ("DuplicateImport", 3)]
        );
    }

    #[test]
    fn import_names_are_read_without_spaces_or_comments_around_dots() {
        // #171: the name was the text after the last `.`, ` TimeUnit` here, so a used import
        // was reported, and --fix deleted it
        let forms = [
            "import java.util.concurrent. TimeUnit",
            "import java.util.concurrent .TimeUnit",
            "import java.util.concurrent . TimeUnit",
            "import java.util.concurrent.\tTimeUnit",
            "import java.util.concurrent.\n    TimeUnit",
            "import java.util.concurrent\n    .TimeUnit",
            "import java. util . concurrent. TimeUnit",
            "import java.util.concurrent./* c */TimeUnit",
            "import java.util.concurrent. // c\n TimeUnit",
        ];
        for f in forms {
            let src = format!("{f}\nTimeUnit t = TimeUnit.SECONDS\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
            // the same import twice, however it is spaced
            let src = format!("{f}\nimport java.util.concurrent.TimeUnit\nTimeUnit t\n");
            assert_eq!(
                rules(&src),
                vec![("DuplicateImport", 2 + f.matches('\n').count())],
                "{src:?}"
            );
            // and unused, named as Groovy reads it
            let vs = lint(&format!("{f}\nx()\n")).unwrap();
            assert_eq!(
                vs.iter().map(|v| v.message.as_str()).collect::<Vec<_>>(),
                vec!["The [java.util.concurrent.TimeUnit] import is never referenced"],
                "{f:?}"
            );
        }
        for (import, usage) in [
            (
                "import static java.util.concurrent.TimeUnit. SECONDS",
                "println SECONDS",
            ),
            (
                "import static java.util.concurrent.TimeUnit .SECONDS",
                "println SECONDS",
            ),
            (
                "import static java.util.concurrent.TimeUnit.\n  SECONDS",
                "println SECONDS",
            ),
            ("import static a.U. getSvc", "svc.go()"),
            ("import a.b. C as D", "D d = null"),
            ("import a.b.C as\n D", "D d = null"),
            ("import a.b .*", "x()"),
            ("import static a.B\n.*", "x()"),
        ] {
            let src = format!("{import}\n{usage}\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
        }
        // an alias, star or static import is a duplicate only of the same alias, star or
        // static import, however spaced
        assert_eq!(
            rules("import a.b .*\nimport a.b.*\nimport static a.b.*\nx()\n"),
            vec![("DuplicateImport", 2)]
        );
        assert_eq!(
            rules("import a.b. C as D\nimport a.b.C as D\nimport a.b.C as E\nD d; E e\n"),
            vec![("DuplicateImport", 2)]
        );
        assert_eq!(
            rules("import static a.B. c\nimport static a.B.c\nimport a.B.c\nc()\n"),
            vec![("DuplicateImport", 2)]
        );
    }

    #[test]
    fn dollar_interpolation_is_a_use() {
        for s in [
            "\"$N\"",
            "\"${M.X} $N\"",
            "\"$N.CONST\"",
            "\"$N.a.b\"",
            "\"\"\"x\n$N y\"\"\"",
            "/x $N y/",
            "$/x $N y/$",
        ] {
            // in parentheses: Groovy reads `println /x/` as a division
            let src = format!("import a.N\nimport a.M\nprintln({s})\nM.go()\n");
            assert_eq!(rules(&src), vec![], "{s}");
        }
        // escaped, or in a string that doesn't interpolate: not a use
        for s in ["\"\\$N\"", "'$N'", "'''$N'''", "\"$Nx\""] {
            let src = format!("import a.N\nprintln {s}\n");
            assert_eq!(rules(&src), vec![("UnusedImport", 1)], "{s}");
        }
    }

    #[test]
    fn interpolation_the_grammar_left_as_text_is_a_use() {
        // gradle ExternalProcessFixture: in this `"""` the `${...}` parses as plain text
        let src = "import a.TestJavaMain\nimport a.TextUtil\nimport a.Gone\ndef s = \"\"\"\n    mainClass.set(\"${TestJavaMain.class.name}\")\n    classpath(\"${TextUtil.escapeString(x)}\")\n    echo \"\\$Gone\"\n\"\"\"\n";
        assert_eq!(rules(src), vec![("UnusedImport", 3)]);
        // a `$` in a single-quoted string is text
        let src = "import a.X\ndef s = '''\n\"${X}\"\n'''\n";
        assert_eq!(rules(src), vec![("UnusedImport", 1)]);
    }

    #[test]
    fn annotation_on_an_import_is_a_use() {
        // #165: JenkinsPipelineUnit's `@Library('x@main') import ...` uses `import ...Library`
        for s in [
            "import a.Gen\n@Gen\nimport b.T\nprintln T.S\n",
            "import a.Gen\n@Gen import b.T\nprintln T.S\n",
            "import a.Gen\n@Gen\nimport static b.T.S\nprintln S\n",
            "import a.Gen\n@a.Gen\nimport b.T\nprintln T.S\n",
            "import a.Lib\n@Lib('x@main') import b.T\nprintln T.S\n",
            "import a.Ann\nimport a.Bar\n@Ann(Bar)\nimport b.T\nprintln T.S\n",
            "import a.Out\nimport a.In\n@Out(@In(Bar.X))\nimport b.T\nimport a.Bar\nprintln T.S\n",
        ] {
            assert_eq!(rules(s), vec![], "{s}");
        }
        // the import's own name and alias still aren't uses of anything
        assert_eq!(
            rules("import a.T\n@Ann\nimport b.T as U\n"),
            vec![("UnusedImport", 1)]
        );
    }

    #[test]
    fn names_used_elsewhere_are_uses() {
        // audited for #165, all already counted: package annotations, annotation arguments,
        // class literals, bounds, throws, instanceof, as, casts, catch, method references
        for (import, usage) in [
            ("import a.Ann", "@Ann\npackage p\nprintln 1"),
            ("import a.Bar", "@Ann(Bar)\nclass X {}"),
            ("import a.Bar", "@Ann(value = Bar.X)\nclass X {}"),
            ("import a.In", "@Out(@In)\nclass X {}"),
            ("import a.Bar", "@Ann(Bar.class)\nclass X {}"),
            ("import a.Bar", "class X<T extends Bar> {}"),
            ("import a.Bar", "def <T extends Bar> void f() {}"),
            ("import a.Bar", "void f() throws Bar {}"),
            ("import a.Bar", "if (x instanceof Bar) {}"),
            ("import a.Bar", "def y = x as Bar"),
            ("import a.Bar", "def y = (Bar) x"),
            ("import a.Bar", "List<Bar> l = []"),
            ("import a.Bar", "try {} catch (Foo | Bar e) {}"),
            ("import a.Bar", "def y = Bar::m"),
            ("import a.Bar", "def f(Bar... b) {}"),
            ("import a.Bar", "class X { void f(@Bar x) {} }"),
            ("import static a.B.c", "c()"),
        ] {
            let src = format!("{import}\n{usage}\n");
            let unused: Vec<_> = rules(&src)
                .into_iter()
                .filter(|r| r.0 == "UnusedImport")
                .collect();
            assert_eq!(unused, vec![], "{src:?}");
        }
    }

    #[test]
    fn unused_aliased_import_names_the_alias() {
        let v = lint("import a.B as C\nimport a.D\n").unwrap();
        let got: Vec<_> = v.iter().map(|v| v.message.as_str()).collect();
        assert_eq!(
            got,
            vec![
                "The alias [C] of the [a.B] import is never referenced",
                "The [a.D] import is never referenced"
            ]
        );
    }

    #[test]
    fn comments_count_whole_words_only() {
        let src = "import a.List\nimport a.Linked\n// Listing, {@link Linked#x}\nclass X {}\n";
        assert_eq!(rules(src), vec![("UnusedImport", 1)]);
    }

    #[test]
    fn unused_import_is_linear_in_comment_text() {
        // a substring search of all comment text per import took over 10 s on this (5 MB) in a
        // release build; one word set takes under 1 s
        let run = |n: usize| {
            let imports: String = (0..n).map(|i| format!("import a.C{i}\n")).collect();
            let src = format!(
                "{imports}{}",
                "// lorem ipsum dolor sit amet\n".repeat(n * 16 / 3)
            );
            assert_eq!(lint(&src).expect("parses").len(), n);
        };
        // up to 10 MB, the largest file linted
        crate::scaling::assert_scales_up_to(
            "imports against comment text",
            run,
            7_500,
            60_000,
            4,
            8,
        );
    }

    #[test]
    fn static_accessor_import_used_as_a_property() {
        // hoist-core ClusterTask: `traceContextService.capture()` resolves to getTraceContextService()
        let src = "import static a.U.getSvc\nimport static a.U.setOut\nimport static a.U.getGone\nimport a.getThing\nsvc.go()\nout = 1\nthing\n";
        assert_eq!(rules(src), vec![("UnusedImport", 3), ("UnusedImport", 4)]);
    }

    #[test]
    fn static_accessor_import_used_under_each_property_name_groovy_reads() {
        // #181 (Alipsa/matrix MatrixDbUtil): Groovy 2.4.21, 3.0.25 and 4.0.24 read each of
        // these uses through the import; a name whose first two letters are capitals stays
        for (import, used) in [
            ("getSCOPES", "SCOPES"),
            ("getDECIMAL_PRECISION", "DECIMAL_PRECISION"),
            ("isACTIVE", "ACTIVE"),
            ("getHTTPHost", "HTTPHost"),
            ("getURL2", "URL2"),
            ("getUrl", "url"),
            ("getUrl", "Url"),
            ("getQ", "q"),
            ("getQ", "Q"),
            ("isB", "b"),
            ("isB", "B"),
            ("getA1", "a1"),
            ("getA1", "A1"),
            ("get_foo", "_foo"),
            ("get_Bar", "_Bar"),
            ("getaB", "aB"),
            ("get\u{c4}pfel", "\u{e4}pfel"),
            ("get\u{c4}pfel", "\u{c4}pfel"),
            ("get$d", "$d"),
        ] {
            let src = format!("import static a.U.{import}\nprintln {used}\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
            let src =
                format!("import static a.U.{import}\nclass W {{ def go() {{ id({used}) }} }}\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
        }
        for (import, used) in [
            ("setURLX", "URLX"),
            ("setName", "Name"),
            ("setUrlY", "urlY"),
        ] {
            let src = format!("import static a.U.{import}\n{used} = 1\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
        }
        // still unused: no form of the property appears, the import isn't static, or there
        // is no property (`get` alone)
        for src in [
            "import static a.U.getSCOPES\nprintln scopesList\n",
            "import static a.U.getHTTPHost\nprintln host\n",
            "import static a.U.isACTIVE\nprintln inactive\n",
            "import static a.U.setURLX\nURL = 1\n",
            "import a.getSCOPES\nprintln SCOPES\n",
            "import static a.U.get\nprintln x\n",
        ] {
            assert_eq!(rules(src), vec![("UnusedImport", 1)], "{src:?}");
        }
    }

    #[test]
    fn aliased_import_checks_alias() {
        assert_eq!(rules("import a.b.Long as L\nL x = null\n"), vec![]);
        assert_eq!(
            rules("import a.b.Long as L\nLong x = null\n"),
            vec![("UnusedImport", 1)]
        );
    }

    #[test]
    fn gstrings() {
        let src = "def a = \"plain\"\ndef b = \"hi ${a}\"\ndef c = 'single'\ndef d = \"it's\"\n";
        assert_eq!(rules(src), vec![("UnnecessaryGString", 1)]);
    }

    #[test]
    fn semicolons() {
        assert_eq!(
            rules("def a = 1;\ndef b = 2; def c = 3\n"),
            vec![("UnnecessarySemicolon", 1)]
        );
        assert_eq!(rules("for (int i = 0; i < 3; i++) { println i }\n"), vec![]);
    }

    #[test]
    fn empty_catch_blocks() {
        let src = "try { x() } catch (Exception e) { }\ntry { x() } catch (Exception ignored) { }\ntry { x() } catch (Exception e) { log(e) }\n";
        assert_eq!(rules(src), vec![("EmptyCatchBlock", 1)]);
    }

    #[test]
    fn empty_blocks() {
        let src = "if (a) { }\nif (a) { x() } else { }\ntry { } finally { }\nwhile (a) { }\nfor (i in 0..2) { }\nif (a) {\n  // intentionally nothing\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyIfStatement", 1),
                ("EmptyElseBlock", 2),
                ("EmptyTryBlock", 3),
                ("EmptyFinallyBlock", 3),
                ("EmptyWhileStatement", 4),
                ("EmptyForStatement", 5),
            ]
        );
        // a condition that calls, assigns or increments does the loop's work
        let src = "while (reader.read() != -1) { }\nwhile ((line = next()) != null) { }\nwhile (i++ < n) { }\nfor (; it.next() != x; ) { }\nwhile (true) { }\nfor (x in [1, 2]) { }\nfor (int i = 0; i < n; i++) { }\nwhile (a.b) { }\n";
        assert_eq!(
            rules(src),
            vec![
                ("ForLoopShouldBeWhileLoop", 4),
                ("EmptyWhileStatement", 5),
                ("EmptyForStatement", 6),
                ("EmptyForStatement", 7),
                ("EmptyWhileStatement", 8),
            ]
        );
        // a for-in loop over anything but a literal may drain an iterator; a classic `for`
        // whose update calls or assigns walks something
        let src = "for (d in data) { }\nfor (d in rows()) { }\nfor (String s : it) { }\nfor (i in 0..<n) { }\nfor (k in [a: 1]) { }\nfor (c in 'abc') { }\nfor (x in [f(), g()]) { }\nfor (n = head; n.next != null; n = n.next) { }\nfor (; it.hasNext(); it.next()) { }\nfor (;;) { }\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyForStatement", 4),
                ("EmptyForStatement", 5),
                ("EmptyForStatement", 6),
                ("EmptyForStatement", 10),
            ]
        );
    }

    #[test]
    fn empty_for_counting_on_locals_is_flagged() {
        // an update that only counts is no work; one that reads a field or calls walks
        let src = "for (int i = 0; i < n; i += 1) { }\nfor (i = 0; i < n; i = i + 1) { }\nfor (int i = n; i > 0; i -= 2) { }\nfor (n = head; n.next != null; n = n.next) {}\nfor (; n; n = n[0]) { }\nfor (; i < n; i = step(i)) { }\nfor (; i < n; i++, j++) { }\nfor (; i < n; i++, it.next()) { }\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyForStatement", 1),
                ("EmptyForStatement", 2),
                ("EmptyForStatement", 3),
                ("EmptyForStatement", 7),
            ]
        );
        // a parenthesized literal is a literal; a call in a range's bounds only sizes it
        let src = "for (i in (0..<n)) {}\nfor (x in ([1,2])) {}\nfor (i in 0..<list.size()) {}\nfor (x in (data)) {}\nfor (x in ([f()])) {}\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyForStatement", 1),
                ("EmptyForStatement", 2),
                ("EmptyForStatement", 3),
            ]
        );
    }

    #[test]
    fn conditionals() {
        let src = "if (x = 1) { f() }\nwhile ((y = next())) { f() }\nif (x == 1) { f() }\nif (true) { f() }\nif (x == x) { f() }\nif (f() == f()) { g() }\n";
        assert_eq!(
            rules(src),
            vec![
                ("AssignmentInConditional", 1),
                ("ConstantIfExpression", 4),
                ("ComparisonWithSelf", 5),
            ]
        );
    }

    #[test]
    fn duplicate_keys_and_cases() {
        let src = "def m = [a: 1, 'a': 2, b: 3, 1: 'x', '1': 'y', \"${k}\": 4, \"${k}\": 5]\nswitch (x) {\n  case 1: f(); break\n  case 2: g(); break\n  case 1: h(); break\n  case 'a': f(); break\n  case '1': f(); break\n  case \"a\": f(); break\n  case '''a''': f(); break\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("DuplicateMapKey", 1),
                ("DuplicateCaseStatement", 5),
                ("DuplicateCaseStatement", 8),
                ("UnnecessaryGString", 8),
                ("DuplicateCaseStatement", 9)
            ]
        );
    }

    #[test]
    fn every_rule_fires_and_every_emitted_rule_is_registered() {
        let src = "import a.B\nimport a.B\nimport c.Unused\ndef s = \"x\";\ntry { f() } catch (e) { }\nif (true) { }\nif (a = 1) { f() } else { }\ntry { } finally { }\nwhile (a) { }\nfor (i in 0..2) { }\ndef m = [a: 1, a: 2]\nswitch (x) { case 1: f(); break\n case 1: g() }\nif (x == x) { f() }\nB.go()\n";
        let mut emitted: HashSet<&str> =
            lint(src).expect("parses").iter().map(|v| v.rule).collect();
        let jenkins = "pipeline {\n agent foo\n agent none\n def x = 1\n bogus { }\n options { parameters([]) }\n stages {\n  stage(\"${x}\") {\n   when { onBranch 'x' }\n   steps { if (x) { } }\n  }\n  stage('a') { }\n  stage('a') { steps { echo 1 } }\n }\n}\n";
        let gradle = "repositories { jcenter() }\ndependencies { compile 'a:b:+' }\ntask t << { }\nversion '1'\ndependencies {\n implementation('a:b:1')\n implementation group: 'a', name: 'b'\n}\n";
        let spock = "class S extends Specification {\n def f() {\n  when:\n  f()\n  then: 'x'\n  where:\n  a | b\n  2 | 3 | 4\n }\n def g() {\n  then:\n  x\n }\n def h() {\n  expect:\n  xs.each { it > 0 }\n }\n}\n";
        let more = "package p\nimport p.Q\nimport java.util.Date\nclass K {\n private int pf\n private void m() { }\n static { }\n { }\n boolean equals(Object o) { true }\n int hashcode() { 1 }\n String toString() { null }\n}\nclass L {\n boolean equals(String s) { true }\n}\ndef g() {\n def u = 1\n return 1\n f()\n}\ntry { f() } finally { return 1; throw new E(); assert x }\nswitch (x) { }\ndef s = [1, 1] as Set\ndef t = true ? 1 : 2\nassert true\nif (1 == 2) { f() }\nif (x != null || x.a) { f() }\nif (x % 2 == 1) { f() }\nl.removeAll(l)\nthrow RuntimeException()\nvoid h() { try { f() } catch (e) { new BadException(e) } }\nDate d = new Q()\ndef bd = new BigDecimal(1.5)\nif (a | b) { f() }\ndef nn = !!x\nsynchronized (l) { }\nfor (; c;) { f() }\nInteger.getInteger('p')\ndef r = (int) Math.random()\ndef io = !x instanceof String\ndef ni = !x in xs\ndef m1 = y % 1\nf(a,,b)\nvoid sl(l) {\n def sx = ''\n for (c in l) { sx << c }\n}\n";
        for (name, src) in [
            ("A.groovy", more),
            ("B.groovy", "// groovyfmt-ignore DeadCode\nprintln 1\n"),
            ("C.groovy", "class D {\n}\n"),
            ("Jenkinsfile", jenkins),
            (
                "Jenkinsfile",
                "pipeline {\n agent any\n environment { A = env.B }\n tools { maven }\n options { retry(1); retry(2) }\n stages {\n  stage('a') {\n   input { ok 'x' }\n   when { not { } }\n   agent any\n   matrix { stages { stage('b') { steps { echo 1 } } } }\n  }\n }\n}\n",
            ),
            ("build.gradle", gradle),
            ("build.gradle", "apply plugin: 'maven'\n"),
            ("S.groovy", spock),
        ] {
            emitted.extend(
                lint_file(Path::new(name), src)
                    .expect("parses")
                    .iter()
                    .map(|v| v.rule),
            );
        }
        let all: HashSet<&str> = RULES.iter().map(|r| r.name).collect();
        assert_eq!(emitted, all);
    }

    #[test]
    fn exemptions_do_not_fire() {
        // nextflow NextflowDSLImpl: `if( (varX=isVariableX(stm.expression)) )`
        let src = "try { f() } catch (Exception ignore) { }\nif (x += 1) { f() }\nif (\"${x}\") { f() }\nenum E { A, B;\n  int v }\nif ((m = s.find(p))) { f() }\n";
        assert_eq!(rules(src), vec![]);
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        // `é` is two bytes; the `;` is the 12th character but the 13th byte
        let v = lint("def s = 'é';\n").expect("parses");
        assert_eq!(
            (v[0].rule, v[0].line, v[0].col, v[0].end_line, v[0].end_col),
            ("UnnecessarySemicolon", 1, 12, 1, 13)
        );
        let v = lint("try { f() } catch (é) {\n}\n").expect("parses");
        assert_eq!(
            (v[0].rule, v[0].line, v[0].col, v[0].end_line, v[0].end_col),
            ("EmptyCatchBlock", 1, 13, 2, 2)
        );
    }

    #[test]
    fn imports_an_include_or_another_import_may_need_are_reported_but_not_fixed() {
        // #192: Hubitat appends the `#include`d library, which uses the file's imports;
        // the last import of a name wins, so removing one can switch the class
        let fixes = |src: &str| -> Vec<(&'static str, usize, bool)> {
            lint(src)
                .expect("parses")
                .into_iter()
                .map(|v| (v.rule, v.line, v.fix.is_some()))
                .collect()
        };
        assert_eq!(
            fixes("import a.B\nimport c.D\nimport c.D\nmetadata { }\n#include x.y\n"),
            vec![
                ("UnusedImport", 1, false),
                ("UnusedImport", 2, false),
                ("DuplicateImport", 3, false)
            ]
        );
        assert_eq!(
            fixes("import p.A\nimport q.A\nimport p.A\nA.v\n"),
            vec![("DuplicateImport", 3, false)]
        );
        assert_eq!(
            fixes("import p.A\nimport p.A\nimport q.B\nA.v\nB.v\n"),
            vec![("DuplicateImport", 2, true)]
        );
    }

    #[test]
    fn empty_loop_body_semicolon_is_never_reported() {
        // The `;` is the loop's whole body: removing it would make `println` the body.
        assert_eq!(lint("while (it.next());\nprintln 'x'\n"), Ok(vec![]));
    }

    #[test]
    fn semicolon_before_a_closure_line_is_kept() {
        // GPars AsyncInvocationWithTimeoutTest: without the `;` the closure becomes an
        // argument of `new DataflowVariable()`
        let src = "final DataflowVariable result = new DataflowVariable();\n{-> result << 20 }.callTimeoutAsync(timeout);\nsleep 3000\n";
        assert_eq!(rules(src), vec![("UnnecessarySemicolon", 2)]);
        // a comment between doesn't matter
        assert_eq!(rules("f();\n// why\n{ -> g() }()\n"), vec![]);
        // bigtop TestBlockRecovery: without the `;`, `x[0]` then `++n` reads as `x[0]++`
        assert_eq!(
            rules("def x = f()[0];\n++n;\nm()\ny();\n--n\n"),
            vec![("UnnecessarySemicolon", 2)]
        );
    }

    #[test]
    fn semicolon_that_keeps_a_method_declaration_away_is_kept() {
        // #211, each checked on Groovy 2.4.21, 3.0.25 and 4.0.24: without the `;`, Groovy 3+
        // reads a top-level type then a call on the next line as a method declaration, an
        // error without a body (`FOO bar()`) and a new method with one (`FOO bar() { 1 }`)
        let semis = |src: &str| {
            lint(src)
                .expect("parses")
                .into_iter()
                .filter(|v| v.rule == "UnnecessarySemicolon")
                .count()
        };
        for src in [
            "FOO;\nbar()\n",
            "Foo;\nbar(int x)\n",
            "FOO;\nbar() { 1 }\n",
            "FOO;\nbar(x) { x }\n",
            "FOO;\nbar()\n{ 1 }\n",
            "a.B;\nbar()\n",
            "java.util.List;\nbar()\n",
            "FOO.BAR;\nbar()\n",
            "Foo.Bar;\nbar(a, b)\n",
            "List<String>;\nbar()\n",
            "Foo<T>;\nbar()\n",
            "String[];\nbar()\n",
            "FOO;\nbar(x)\n",
            "FOO;\nbar(x = 1)\n",
            "FOO;\nbar(Foo)\n",
            "FOO;\nbar(Foo x, int y)\n",
            "FOO;\nbar(it)\n",
            "FOO;\n'bar'()\n",
            "FOO;\n\"bar\"()\n",
            "FOO;\nBar()\n",
            "FOO;\nyield()\n",
            "FOO; // c\n\n// d\nbar()\n",
            "FOO;;\nbar()\n",
            "FOO /* c */;\nbar()\n",
            "FOO;\nbar ()\n",
            "class A {}\nFOO;\nbar()\n",
            // conservative: Groovy reads these two statements, as no argument can be a parameter
            "FOO;\nbar(1)\n",
            "FOO;\nbar(x.y)\n",
        ] {
            assert_eq!(semis(src), 0, "{src:?}");
        }
        // same meaning without the `;` on all three
        for src in [
            "foo;\nbar()\n",
            "a.b;\nbar()\n",
            "A.b;\nbar()\n",
            "this.FOO;\nbar()\n",
            "super.FOO;\nbar()\n",
            "FOO.class;\nbar()\n",
            "L: FOO;\nbar()\n",
            "x = FOO;\nbar()\n",
            "foo();\nbar()\n",
            "FOO();\nbar()\n",
            "String x;\nbar()\n",
            "def x;\nbar()\n",
            "FOO;\nbar x\n",
            "FOO;\nbar\n",
            "FOO;\nbar = 1\n",
            "FOO;\nbar {}\n",
            "FOO;\ndef bar() {}\n",
            // not at the top level, where no method can be declared
            "def m() {\n  FOO;\n  bar()\n}\n",
            "c = {\n  FOO;\n  bar()\n}\n",
            "if (c) {\n  FOO;\n  bar()\n}\n",
        ] {
            assert_eq!(semis(src), 1, "{src:?}");
        }
    }

    #[test]
    fn annotated_imports_are_never_unused() {
        // a Jenkins shared library or a Grape dependency is loaded by the annotation
        let src =
            "@Library('lib@master')\nimport org.foo.Zot\n@Grab('a:b:1') import c.D\nimport e.F\n";
        let v = lint(src).expect("parses");
        assert_eq!(
            v.iter().map(|v| (v.rule, v.line)).collect::<Vec<_>>(),
            vec![("UnusedImport", 4)]
        );
        let dup = lint("@Grab('a:b:1') import c.D\n@Grab('a:b:1') import c.D\nD.go()\n").unwrap();
        assert_eq!(dup.len(), 1);
        assert_eq!((dup[0].rule, dup[0].fix.clone()), ("DuplicateImport", None));
    }

    #[test]
    fn gorm_where_queries_compare_a_property_with_a_variable() {
        let cfg = |src: &str| -> Vec<(&'static str, usize)> {
            rules(src)
                .into_iter()
                .filter(|(r, _)| *r == "ComparisonWithSelf")
                .collect()
        };
        // grails-core WhereQueryVariableScopeService, EmbeddedWhereClauseSpec, WhereMethodSpec
        let src = "def f(String title) {\n    Book.where {\n        title == title\n    }.list()\n}\ninterface S {\n    @Where({ contexts { n == n } })\n    List f(String n)\n}\nPerson.where { age > age && pets { age > age } }\nwhere { a == a }\n";
        assert_eq!(cfg(src), vec![]);
        // other closures still count
        let src =
            "xs.findAll { title == title }\ndef where(x) { x == x }\nBook.where(title == title)\n";
        assert_eq!(
            cfg(src),
            vec![
                ("ComparisonWithSelf", 1),
                ("ComparisonWithSelf", 2),
                ("ComparisonWithSelf", 3)
            ]
        );
    }

    #[test]
    fn explain_examples_show_their_rule_and_fixable_matches_the_fixes() {
        for r in RULES {
            // the domain rules look at the file name
            let path = match &r.name[..r.name.len().min(6)] {
                "Jenkin" => "Jenkinsfile",
                "Gradle" => "build.gradle",
                _ => "MathSpec.groovy",
            };
            let fired = |src: &str| {
                lint_file(Path::new(path), src)
                    .expect(src)
                    .into_iter()
                    .filter(|v| v.rule == r.name)
                    .collect::<Vec<_>>()
            };
            let bad = fired(r.bad);
            assert!(!bad.is_empty(), "{}: bad example doesn't fire", r.name);
            assert!(fired(r.good).is_empty(), "{}: good example fires", r.name);
            assert_eq!(bad.iter().all(|v| v.fix.is_some()), r.fixable, "{}", r.name);
            assert!(!r.why.is_empty(), "{}", r.name);
        }
        for name in PARTLY_FIXABLE {
            let r = RULES.iter().find(|r| r.name == *name).expect(name);
            assert!(!r.fixable, "{name}");
        }
    }

    #[test]
    fn docs_rule_counts_match_rules() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        // the crates.io package has no docs/, so the test runs in a checkout only
        if !root.join("docs").is_dir() {
            return;
        }
        let read = |p: &str| std::fs::read_to_string(root.join(p)).expect(p);
        let (n, on) = (RULES.len(), RULES.iter().filter(|r| r.default).count());
        let want = format!("{n} rules, {on} on by default");
        assert!(read("README.md").contains(&want), "README.md: {want}");
        let want = format!("has {n} rules, {on} of them on by default");
        assert!(read("docs/lint.md").contains(&want), "docs/lint.md: {want}");
    }

    #[test]
    fn unparseable_is_not_linted() {
        assert_eq!(lint("def x = {\n"), Err("parse error"));
    }
}
