//! Gradle build scripts (`*.gradle`): syntax that current Gradle versions removed.
//! Versions and replacements are from the upgrade guides:
//! https://docs.gradle.org/current/userguide/upgrading_version_6.html (compile, runtime,
//! testCompile, testRuntime removed in 7.0; jcenter() deprecated),
//! https://docs.gradle.org/current/userguide/upgrading_major_version_9.html (jcenter() removed
//! in 9.0), https://docs.gradle.org/current/userguide/upgrading_version_4.html (`<<` task
//! actions deprecated in 4.10, gone in 5.0).

use super::jenkins::{call, statements};
use super::{Violation, at, constant_value, has_interpolation};
use crate::text;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::Node;

/// Removed configuration, replacement.
const REMOVED: &[(&str, &str)] = &[
    (
        "compile",
        "implementation (or api for dependencies your API exposes)",
    ),
    ("runtime", "runtimeOnly"),
    ("testCompile", "testImplementation"),
    ("testRuntime", "testRuntimeOnly"),
];

pub(super) fn check(path: &Path, src: &str, root: Node, v: &mut Vec<Violation>) {
    if path.extension().is_none_or(|e| e != "gradle") {
        return;
    }
    // A build can still declare its own configuration called `compile`: then it's valid.
    let mut declared = HashSet::new();
    let mut deps = Vec::new();
    crate::preorder(root, |n| {
        match n.kind() {
            "method_invocation" => jcenter(src, n, v),
            "command_chain" => {
                jcenter(src, n, v);
                if n.child_by_field_name("receiver")
                    .is_some_and(|r| text(r, src) == "task")
                {
                    left_shift(n, v);
                }
            }
            "expression_statement" if text(n, src) == "task" => left_shift(n, v),
            _ => {}
        }
        if let Some(c) = call(n, src)
            && let Some(b) = c.body
        {
            match c.name {
                "dependencies" => deps.push(b),
                "configurations" => {
                    for s in statements(b) {
                        let t = text(s, src);
                        let end = t
                            .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                            .unwrap_or(t.len());
                        declared.insert(t[..end].to_string());
                    }
                }
                _ => {}
            }
        }
        // configurations.create('compile'), maybeCreate, register
        if n.kind() == "method_invocation"
            && n.child_by_field_name("function").is_some_and(|f| {
                let t = text(f, src);
                t.ends_with(".create") || t.ends_with(".maybeCreate") || t.ends_with(".register")
            })
            && let Some(a) = n
                .child_by_field_name("arguments")
                .and_then(|a| a.named_child(0))
            && let Some(name) = constant_value(a, src)
        {
            declared.insert(name);
        }
        true
    });
    // read only for a file that would get a report
    let old_gradle = std::cell::OnceCell::new();
    for b in deps {
        for s in statements(b) {
            let Some(c) = call(s, src) else { continue };
            if c.args.is_empty() {
                continue;
            }
            if let Some((_, new)) = REMOVED.iter().find(|(old, _)| *old == c.name)
                && !declared.contains(c.name)
                && !*old_gradle.get_or_init(|| wrapper_major(path).is_some_and(|m| m < 7))
            {
                v.push(at(
                    s,
                    "GradleRemovedConfiguration",
                    format!(
                        "The `{}` configuration was removed in Gradle 7.0; use {new}",
                        c.name
                    ),
                ));
            }
            dynamic_version(src, &c, v);
        }
    }
}

/// The major Gradle version of the nearest `gradle/wrapper/gradle-wrapper.properties` in
/// the script's directory or above it (`distributionUrl=.../gradle-6.9-bin.zip` is 6). A
/// build pinned below 7.0 runs a Gradle that still has the removed configurations.
fn wrapper_major(path: &Path) -> Option<u32> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let props = dir.ancestors().find_map(|d| {
        std::fs::read_to_string(d.join("gradle/wrapper/gradle-wrapper.properties")).ok()
    })?;
    major_version(&props)
}

fn major_version(props: &str) -> Option<u32> {
    let url = props
        .lines()
        .find_map(|l| l.trim().strip_prefix("distributionUrl"))?;
    url.rsplit("gradle-")
        .next()?
        .split(['.', '-'])
        .next()?
        .parse()
        .ok()
}

/// `jcenter()` or `jcenter { ... }` in a repositories block.
fn jcenter(src: &str, n: Node, v: &mut Vec<Violation>) {
    let f = n
        .child_by_field_name("function")
        .or_else(|| n.child_by_field_name("receiver"));
    if f.is_some_and(|f| matches!(text(f, src), "jcenter" | "repositories.jcenter")) {
        v.push(at(
            n,
            "GradleJcenter",
            "jcenter() was deprecated in Gradle 7.0 and removed in 9.0, and JCenter is read-only; use mavenCentral()".into(),
        ));
    }
}

/// `task hello << { ... }`. The grammar reads it as two statements, `task` and
/// `hello << { ... }`, so this looks at the statement after a bare `task` on the same line;
/// `task 'hello' << { }` is one command whose argument is the `<<`.
fn left_shift(n: Node, v: &mut Vec<Violation>) {
    let e = if n.kind() == "command_chain" {
        n.child_by_field_name("argument")
    } else {
        n.next_named_sibling()
            .filter(|next| {
                next.kind() == "expression_statement"
                    && next.start_position().row == n.end_position().row
            })
            .and_then(|next| next.named_child(0))
    };
    let Some(e) = e else { return };
    if e.kind() == "binary_expression"
        && e.child_by_field_name("operator")
            .is_some_and(|o| o.kind() == "<<")
        && e.child_by_field_name("right")
            .is_some_and(|r| r.kind() == "closure")
    {
        v.push(at(
            n,
            "GradleLeftShiftTask",
            "`task name << { }` was removed in Gradle 5.0; write task name { doLast { ... } }"
                .into(),
        ));
    }
}

/// A `+` or `latest.*` version in `group:name:version` or `version: '...'`.
fn dynamic_version(src: &str, c: &super::jenkins::Call, v: &mut Vec<Violation>) {
    for a in &c.args {
        let (lit, version) = match a.kind() {
            "string_literal" => (*a, 2),
            "named_argument"
                if a.child_by_field_name("key")
                    .is_some_and(|k| text(k, src) == "version") =>
            {
                match a.child_by_field_name("value") {
                    Some(val) if val.kind() == "string_literal" => (val, 0),
                    _ => continue,
                }
            }
            _ => continue,
        };
        if has_interpolation(lit) {
            continue;
        }
        let Some(s) = constant_value(lit, src) else {
            continue;
        };
        let s = s.split('@').next().unwrap_or("");
        let Some(ver) = s.split(':').nth(version) else {
            continue;
        };
        if ver.ends_with('+') || ver.starts_with("latest.") {
            v.push(at(
                lit,
                "GradleDynamicVersion",
                format!("Dynamic version `{ver}` makes builds unrepeatable; pin a version (or use a lock file)"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::lint::lint_file;
    use std::path::Path;

    fn rules(name: &str, src: &str) -> Vec<(&'static str, usize)> {
        lint_file(Path::new(name), src)
            .expect("parses")
            .into_iter()
            .filter(|v| v.rule.starts_with("Gradle"))
            .map(|v| (v.rule, v.line))
            .collect()
    }

    #[test]
    fn removed_configurations() {
        let src = "dependencies {\n    compile 'a:b:1'\n    compile project(':x')\n    testCompile('a:b:1') { exclude group: 'c' }\n    runtime group: 'a', name: 'b', version: '1'\n    testRuntime files('x.jar')\n    implementation 'a:b:1'\n    compile\n}\nconfigurations { compileClasspath }\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![
                ("GradleRemovedConfiguration", 2),
                ("GradleRemovedConfiguration", 3),
                ("GradleRemovedConfiguration", 4),
                ("GradleRemovedConfiguration", 5),
                ("GradleRemovedConfiguration", 6),
            ]
        );
        // only in Gradle scripts, and not where the build declares the configuration itself
        assert_eq!(rules("Build.groovy", src), vec![]);
        for decl in [
            "configurations { compile }",
            "configurations {\n compile.extendsFrom x\n}",
            "configurations.create('compile')",
        ] {
            let src = format!("{decl}\ndependencies {{\n    compile 'a:b:1'\n}}\n");
            assert_eq!(rules("build.gradle", &src), vec![], "{decl}");
        }
        // not outside a dependencies block
        assert_eq!(
            rules("build.gradle", "task compile { }\ncompile 'x'\n"),
            vec![]
        );
    }

    #[test]
    fn removed_configurations_under_an_old_wrapper() {
        // the nearest wrapper decides: a Gradle below 7.0 still has `compile`
        let root = std::env::temp_dir().join(format!("groovyfmt-wrapper-{}", std::process::id()));
        let sub = root.join("sub");
        std::fs::create_dir_all(sub.join("gradle/wrapper")).unwrap();
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let wrapper = |dir: &Path, v: &str| {
            std::fs::write(
                dir.join("gradle/wrapper/gradle-wrapper.properties"),
                format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n"),
            )
            .unwrap()
        };
        let src = "dependencies {\n    compile 'a:b:1'\n}\n";
        let run = |p: &Path| rules(p.join("build.gradle").to_str().unwrap(), src).len();
        wrapper(&root, "8.5");
        wrapper(&sub, "6.9");
        assert_eq!((run(&root), run(&sub), run(&sub.join("x"))), (1, 0, 0));
        wrapper(&sub, "7.0-rc-1");
        assert_eq!(run(&sub), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn jcenter() {
        let src = "repositories {\n    jcenter()\n    mavenCentral()\n}\nbuildscript { repositories { jcenter { url 'x' } } }\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![("GradleJcenter", 2), ("GradleJcenter", 5)]
        );
    }

    #[test]
    fn left_shift_tasks() {
        let src = "task hello << {\n    println 'hi'\n}\ntask copy(type: Copy) << { }\ntask 'quoted' << { }\ntask ok {\n    doLast { }\n}\nlist << { it }\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![
                ("GradleLeftShiftTask", 1),
                ("GradleLeftShiftTask", 4),
                ("GradleLeftShiftTask", 5),
            ]
        );
    }

    #[test]
    fn dynamic_versions() {
        let src = "dependencies {\n    implementation 'a:b:1.+'\n    implementation 'a:b:latest.release'\n    implementation group: 'a', name: 'b', version: '+'\n    implementation 'a:b:1.2'\n    implementation \"a:b:${v}\"\n    implementation 'a:b:1.0@jar'\n}\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![
                ("GradleDynamicVersion", 2),
                ("GradleDynamicVersion", 3),
                ("GradleDynamicVersion", 4),
            ]
        );
    }
}
