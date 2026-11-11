//! UnnecessaryGroovyImport and ImportFromSamePackage (CodeNarc's `imports` rules).
//!
//! How Groovy resolves a simple class name (`ResolveVisitor.resolve`): nested classes, then
//! classes of the file, then single-type imports, then the file's package, then static and
//! star imports, then the default imports. So an import of the file's own package is never
//! needed (the package lookup right after finds the same class), and deleting it is safe;
//! an import of a default package can be needed, to beat a same-package or star-imported
//! class of the same name, which one file can't show, so that rule reports but doesn't fix.
//! Only capitalized names: Groovy resolves a lowercase class name through imports alone.

use super::{Violation, at, own_lines};
use crate::text;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::Node;

/// Packages Groovy imports into every file, and the two single classes.
const DEFAULT_PACKAGES: &[&str] = &[
    "java.lang",
    "java.util",
    "java.io",
    "java.net",
    "groovy.lang",
    "groovy.util",
];
const DEFAULT_CLASSES: &[&str] = &["java.math.BigDecimal", "java.math.BigInteger"];

/// The node kinds `check` looks at: the root, and the declarations of classes.
pub(super) const KINDS: &[&str] = &[
    "source_file",
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "trait_declaration",
    "record_declaration",
    "annotation_type_declaration",
];

/// `nodes`: the nodes of the kinds in `KINDS`, in preorder, so the root first.
pub(super) fn check(path: &Path, src: &str, nodes: &[Node], v: &mut Vec<Violation>) {
    // Gradle adds star imports of its own (`org.gradle.api.tasks.*` holds an `Optional`), so
    // in a build script an import of a default package can be what picks the JDK class
    let gradle = path.extension().is_some_and(|e| e == "gradle");
    let Some(&root) = nodes.first() else { return };
    let mut c = root.walk();
    let top: Vec<Node> = root.named_children(&mut c).collect();
    let package = top
        .iter()
        .find(|n| n.kind() == "package_declaration")
        .and_then(|p| p.child_by_field_name("name"))
        .map(|n| text(n, src));
    let imports: Vec<Node> = top
        .iter()
        .copied()
        .filter(|n| n.kind() == "import_declaration")
        .collect();
    if imports.is_empty() {
        return;
    }
    // a class this file declares, at any depth, shadows an import of that name
    let mut declared = HashSet::new();
    for n in &nodes[1..] {
        if let Some(name) = n.child_by_field_name("name") {
            declared.insert(text(name, src));
        }
    }
    // star imports other than the defaults can hold a class of the same simple name
    let other_stars = imports.iter().any(|d| {
        let t = text(*d, src);
        let name = d.child_by_field_name("name").map_or("", |n| text(n, src));
        !t.contains("static")
            && (t.trim_end_matches(';').trim_end().ends_with(".*"))
            && !DEFAULT_PACKAGES.contains(&name)
            && Some(name) != package
    });
    for d in imports {
        let t = text(d, src);
        // `import static`, `import a.B as C` and `@Grab(...) import x` are there for more
        // than the name
        if t.starts_with('@')
            || d.child_by_field_name("alias").is_some()
            || t.split_whitespace().nth(1) == Some("static")
        {
            continue;
        }
        let Some(name) = d.child_by_field_name("name").map(|n| text(n, src)) else {
            continue;
        };
        let star = t.trim_end_matches(';').trim_end().ends_with(".*");
        let (pkg, class) = if star {
            (name, None)
        } else {
            match name.rsplit_once('.') {
                Some((p, c)) if c.starts_with(|c: char| c.is_ascii_uppercase()) => (p, Some(c)),
                _ => continue,
            }
        };
        let shown = if star {
            format!("{name}.*")
        } else {
            name.to_string()
        };
        if Some(pkg) == package {
            let fix = own_lines(src, d).filter(|_| class.is_none_or(|c| !declared.contains(c)));
            v.push(Violation {
                fix,
                ..at(
                    d,
                    "ImportFromSamePackage",
                    format!("The [{shown}] import is from this file's own package, whose classes need no import; remove it"),
                )
            });
        } else if (DEFAULT_PACKAGES.contains(&pkg) || DEFAULT_CLASSES.contains(&name))
            && class.is_none_or(|c| !declared.contains(c))
            && (star || !other_stars)
            && !gradle
        {
            v.push(at(
                d,
                "UnnecessaryGroovyImport",
                format!("The [{shown}] import is unnecessary: Groovy imports it by default; remove it unless it is there to beat a class of the same name in this package"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::lint::lint;

    fn rules(src: &str) -> Vec<(&'static str, usize, bool)> {
        lint(src)
            .expect("parses")
            .into_iter()
            .filter(|v| v.rule != "UnusedImport")
            .map(|v| (v.rule, v.line, v.fix.is_some()))
            .collect()
    }

    #[test]
    fn default_imports() {
        let src = "import java.util.List\nimport java.util.*\nimport java.math.BigDecimal\nimport java.math.RoundingMode\nimport java.util.Map.Entry\nimport java.util.List as L\nimport static java.lang.Math.max\nimport java.util.concurrent.TimeUnit\nimport groovy.lang.Closure\n";
        assert_eq!(
            rules(src),
            vec![
                ("UnnecessaryGroovyImport", 1, false),
                ("UnnecessaryGroovyImport", 2, false),
                ("UnnecessaryGroovyImport", 3, false),
                ("UnnecessaryGroovyImport", 9, false),
            ]
        );
        // a star import or a class of this file could hold a class of the same name
        assert_eq!(rules("import a.*\nimport java.util.Date\n"), vec![]);
        assert_eq!(
            rules("import java.util.Date\nclass A { class Date { } }\n"),
            vec![]
        );
        // Gradle's own default imports include `org.gradle.api.tasks.Optional`
        let gradle = crate::lint::lint_file(
            std::path::Path::new("settings.gradle"),
            "import java.util.Optional\nOptional.empty()\n",
        )
        .unwrap();
        assert!(gradle.is_empty(), "{gradle:?}");
        let star = crate::lint::lint("import java.util.*\n").unwrap();
        assert!(
            star[0].message.contains("[java.util.*]"),
            "{}",
            star[0].message
        );
    }

    #[test]
    fn same_package() {
        let src = "package a.b\nimport a.b.C\nimport a.b.*\nimport a.b.c.D\nimport a.b.lower\nimport a.b.E as F\nimport static a.b.C.x\nimport a.b.Nested\nclass Z { class Nested { } }\n";
        assert_eq!(
            rules(src),
            vec![
                ("ImportFromSamePackage", 2, true),
                ("ImportFromSamePackage", 3, true),
                ("ImportFromSamePackage", 8, false),
            ]
        );
        // no package: nothing is the same package
        assert_eq!(rules("import C\n"), vec![]);
    }
}
