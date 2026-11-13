//! Gradle build scripts (`*.gradle`): syntax that current Gradle versions removed.
//! Versions and replacements are from the upgrade guides:
//! https://docs.gradle.org/current/userguide/upgrading_version_6.html (compile, runtime,
//! testCompile, testRuntime removed in 7.0; jcenter() deprecated),
//! https://docs.gradle.org/current/userguide/upgrading_major_version_9.html (jcenter() removed
//! in 9.0), https://docs.gradle.org/current/userguide/upgrading_version_4.html (`<<` task
//! actions deprecated in 4.10, gone in 5.0), https://docs.gradle.org/current/userguide/upgrading_version_5.html
//! (findbugs, jdepend, osgi and announce plugins removed in 6.0),
//! https://docs.gradle.org/current/userguide/upgrading_version_7.html (archive task
//! properties removed in 8.0).

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
    // GradleRemovedApi: (node, message, the Gradle major that removed it)
    let mut removed: Vec<(Node, String, u32)> = Vec::new();
    let mut conf_refs = Vec::new();
    crate::preorder(root, |n| {
        removed_api(src, n, &mut removed, &mut conf_refs);
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
    for (n, name) in conf_refs {
        if !declared.contains(name) {
            let new = REMOVED
                .iter()
                .find(|(old, _)| *old == name)
                .map_or("", |r| r.1);
            removed.push((
                n,
                format!("The `{name}` configuration was removed in Gradle 7.0, so `configurations.{name}` fails; use {new}, or a resolvable one such as runtimeClasspath"),
                7,
            ));
        }
    }
    if removed.is_empty() {
        return;
    }
    let major = wrapper_major(path);
    for (n, msg, since) in removed {
        // quiet under a wrapper pinning a Gradle that still has it; without a wrapper, a
        // current Gradle (8 or 9, which have none of these) runs the build
        if major.is_none_or(|m| m >= since) {
            v.push(at(n, "GradleRemovedApi", msg));
        }
    }
}

/// Core plugins removed from Gradle: id, the major that removed it, the replacement. From
/// the upgrade guides for 5.x to 6.0 and 6.x to 7.0.
const REMOVED_PLUGINS: &[(&str, u32, &str)] = &[
    ("maven", 7, "use the maven-publish plugin"),
    (
        "findbugs",
        6,
        "use the SpotBugs plugin (com.github.spotbugs)",
    ),
    (
        "jdepend",
        6,
        "use a community plugin from the Gradle Plugin Portal",
    ),
    (
        "osgi",
        6,
        "use a community OSGi plugin such as biz.aQute.bnd.builder",
    ),
    ("announce", 6, "use a community notification plugin"),
    (
        "build-announcements",
        6,
        "use a community notification plugin",
    ),
    (
        "compare-gradle-builds",
        6,
        "use a Build Scan to compare builds",
    ),
    ("java-lang", 7, "use the java-library plugin"),
    ("scala-lang", 7, "use the scala plugin"),
    ("junit-test-suite", 7, "use the java-library plugin"),
    ("jvm-component", 7, "use the java-library plugin"),
    ("jvm-resources", 7, "use the java-library plugin"),
    (
        "coffeescript-base",
        7,
        "use a plugin from the Gradle Plugin Portal",
    ),
    ("envjs", 7, "use a plugin from the Gradle Plugin Portal"),
    (
        "javascript-base",
        7,
        "use a plugin from the Gradle Plugin Portal",
    ),
    ("jshint", 7, "use a plugin from the Gradle Plugin Portal"),
    ("rhino", 7, "use a plugin from the Gradle Plugin Portal"),
];

/// Properties of archive tasks (Jar, Zip, ...) removed in Gradle 8.0, and their
/// replacements. `version` is left out: in a task block it falls through to the project's
/// own `version`, which is valid.
const ARCHIVE_PROPS: &[(&str, &str)] = &[
    ("appendix", "archiveAppendix"),
    ("archiveName", "archiveFileName"),
    ("baseName", "archiveBaseName"),
    ("classifier", "archiveClassifier"),
    ("destinationDir", "destinationDirectory"),
    ("extension", "archiveExtension"),
];
/// Task types that extend AbstractArchiveTask.
const ARCHIVE_TYPES: &[&str] = &[
    "Jar",
    "Zip",
    "Tar",
    "War",
    "Ear",
    "ShadowJar",
    "BootJar",
    "BootWar",
];
/// Archive tasks that core plugins (and Spring Boot's and Shadow's) create under these names.
const ARCHIVE_TASKS: &[&str] = &[
    "jar",
    "war",
    "ear",
    "sourcesJar",
    "javadocJar",
    "distZip",
    "distTar",
    "bootJar",
    "bootWar",
    "shadowJar",
];
/// Blocks a task configuration block can sit in and still mean the task.
const TASK_SCOPES: &[&str] = &[
    "allprojects",
    "subprojects",
    "project",
    "configure",
    "afterEvaluate",
    "tasks",
];

/// The last part of a dotted name: `ShadowJar` of `com.github.jengelman.gradle.plugins.shadow.tasks.ShadowJar`.
fn last(t: &str) -> &str {
    t.rsplit('.').next().unwrap_or(t)
}

fn removed_api<'a>(
    src: &'a str,
    n: Node<'a>,
    out: &mut Vec<(Node<'a>, String, u32)>,
    conf_refs: &mut Vec<(Node<'a>, &'a str)>,
) {
    // configurations.compile
    if n.kind() == "field_access"
        && let (Some(o), Some(f)) = (
            n.child_by_field_name("object"),
            n.child_by_field_name("field"),
        )
        && last(text(o, src)) == "configurations"
        && REMOVED.iter().any(|(old, _)| *old == text(f, src))
    {
        conf_refs.push((n, text(f, src)));
    }
    // apply plugin: 'maven', apply(plugin: 'maven')
    if n.kind() == "named_argument"
        && n.child_by_field_name("key")
            .is_some_and(|k| text(k, src) == "plugin")
        && let Some(val) = n.child_by_field_name("value")
        && let Some(id) = constant_value(val, src).filter(|_| val.kind() == "string_literal")
        && let Some((_, since, new)) = REMOVED_PLUGINS.iter().find(|p| p.0 == id)
        && n.parent()
            .and_then(|p| {
                if p.kind() == "argument_list" {
                    p.parent()
                } else {
                    Some(p)
                }
            })
            .and_then(|p| {
                p.child_by_field_name("receiver")
                    .or_else(|| p.child_by_field_name("function"))
            })
            .is_some_and(|f| text(f, src) == "apply")
    {
        out.push((
            n,
            format!("The `{id}` plugin was removed in Gradle {since}.0; {new}"),
            *since,
        ));
    }
    if let Some(body) = archive_body(src, n) {
        for s in statements(body) {
            let name = match call(s, src) {
                Some(c) if !c.args.is_empty() && c.body.is_none() => Some(c.name),
                _ => s
                    .named_child(0)
                    .filter(|a| {
                        s.kind() == "expression_statement"
                            && a.kind() == "assignment_expression"
                            && a.child_by_field_name("operator")
                                .is_some_and(|o| o.kind() == "=")
                    })
                    .and_then(|a| a.child_by_field_name("left"))
                    .filter(|l| l.kind() == "identifier")
                    .map(|l| text(l, src)),
            };
            if let Some((old, new)) = name.and_then(|x| ARCHIVE_PROPS.iter().find(|p| p.0 == x)) {
                out.push((
                    s,
                    format!(
                        "The `{old}` property of archive tasks was removed in Gradle 8.0; use {new}"
                    ),
                    8,
                ));
            }
        }
    }
    let Some(c) = call(n, src) else { return };
    // plugins { id 'maven' }
    if c.name == "id"
        && let [a] = c.args.as_slice()
        && a.kind() == "string_literal"
        && let Some(id) = constant_value(*a, src)
        && let Some((_, since, new)) = REMOVED_PLUGINS.iter().find(|p| p.0 == id)
        && owners(n, src).next() == Some("plugins")
    {
        out.push((
            n,
            format!("The `{id}` plugin was removed in Gradle {since}.0; {new}"),
            *since,
        ));
    }
    // a build may declare its own task of that name
    if c.name == "uploadArchives"
        && c.body.is_some()
        && ![
            "task uploadArchives",
            "'uploadArchives'",
            "\"uploadArchives\"",
        ]
        .iter()
        .any(|d| src.contains(d))
    {
        out.push((
            n,
            "The uploadArchives task was removed in Gradle 7.0; publish with the maven-publish or ivy-publish plugin".into(),
            7,
        ));
    }
}

/// The names of the calls whose closures hold `n`, innermost first.
fn owners<'a>(n: Node<'a>, src: &'a str) -> impl Iterator<Item = &'a str> {
    std::iter::successors(n.parent(), |p| p.parent())
        .filter(|p| p.kind() == "closure")
        .map(move |cl| {
            let p = cl.parent();
            p.and_then(|p| call(p, src))
                .or_else(|| p.and_then(|p| p.parent()).and_then(|g| call(g, src)))
                .map(|c| c.name)
                .or_else(|| {
                    p.filter(|p| p.kind() == "method_invocation")
                        .and_then(|p| p.child_by_field_name("function"))
                        .map(|f| last(text(f, src)))
                })
                .unwrap_or("")
        })
}

/// The configuration block of an archive task: `jar { }` (where it means the task),
/// `task x(type: Jar) { }`, `tasks.register('x', Zip) { }`, `tasks.withType(Jar) { }` or
/// `tasks.withType(Jar).configureEach { }`.
fn archive_body<'a>(src: &'a str, n: Node<'a>) -> Option<Node<'a>> {
    let typed = |m: Node| {
        let Some(a) = m.child_by_field_name("arguments") else {
            return false;
        };
        let mut c = a.walk();
        a.named_children(&mut c).any(|x| {
            let x = if x.kind() == "named_argument"
                && x.child_by_field_name("key")
                    .is_some_and(|k| text(k, src) == "type")
            {
                x.child_by_field_name("value")
            } else {
                Some(x)
            };
            x.is_some_and(|x| {
                matches!(x.kind(), "identifier" | "field_access")
                    && ARCHIVE_TYPES.contains(&last(text(x, src)))
            })
        })
    };
    let in_scope = || owners(n, src).all(|o| TASK_SCOPES.contains(&o));
    match n.kind() {
        "method_invocation" => {
            let body = n.child_by_field_name("closure")?;
            let f = n.child_by_field_name("function")?;
            let ok = typed(n)
                || (f.kind() == "field_access"
                    && f.child_by_field_name("field")
                        .is_some_and(|x| text(x, src) == "configureEach")
                    && f.child_by_field_name("object")
                        .is_some_and(|o| o.kind() == "method_invocation" && typed(o)))
                || (f.kind() == "identifier"
                    && ARCHIVE_TASKS.contains(&text(f, src))
                    && n.child_by_field_name("arguments")
                        .is_none_or(|a| a.named_child_count() == 0)
                    && in_scope());
            ok.then_some(body)
        }
        "command_chain" => {
            let c = call(n, src)?;
            (ARCHIVE_TASKS.contains(&c.name) && c.args.is_empty() && in_scope())
                .then_some(c.body)
                .flatten()
        }
        _ => None,
    }
}

/// The major Gradle version of the nearest `gradle/wrapper/gradle-wrapper.properties` in
/// the script's directory or above it (`distributionUrl=.../gradle-6.9-bin.zip` is 6). A
/// build pinned below 7.0 runs a Gradle that still has the removed configurations. Like the
/// search for `.groovyfmt.toml`, the walk goes up to the filesystem root, so a wrapper in a
/// directory above the project counts when the project has none. A path that doesn't exist
/// (an unsaved editor buffer) is walked as written.
fn wrapper_major(path: &Path) -> Option<u32> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let props = dir
        .ancestors()
        .map(|d| d.join("gradle/wrapper/gradle-wrapper.properties"))
        .find(|p| p.is_file())?;
    major_version(&read_small(&props)?)
}

/// The real file is a few hundred bytes.
const MAX_WRAPPER_BYTES: u64 = 64 * 1024;

/// `p` if it is a regular file of at most `MAX_WRAPPER_BYTES`, in UTF-8. A FIFO or device
/// is never opened (opening a FIFO waits for a writer); one swapped in between the check and
/// the open is the race the source reads accept too (see `write_in_place`).
fn read_small(p: &Path) -> Option<String> {
    use std::io::Read;
    let ok = |m: std::fs::Metadata| m.is_file() && m.len() <= MAX_WRAPPER_BYTES;
    if !std::fs::metadata(p).is_ok_and(ok) {
        return None;
    }
    let f = std::fs::File::open(p).ok()?;
    if !f.metadata().is_ok_and(ok) {
        return None;
    }
    let mut bytes = Vec::new();
    f.take(MAX_WRAPPER_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_WRAPPER_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
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
        // an oversized file isn't read: the report stays
        let props = sub.join("gradle/wrapper/gradle-wrapper.properties");
        let mut big = b"distributionUrl=https\\://x/gradle-6.9-bin.zip\n".to_vec();
        big.resize(super::MAX_WRAPPER_BYTES as usize + 1, b'#');
        std::fs::write(&props, big).unwrap();
        assert_eq!(run(&sub), 1);
        // a FIFO is never opened (the open would wait for a writer, hanging this test): the
        // walk passes it by and finds the 8.5 wrapper above
        #[cfg(unix)]
        {
            std::fs::remove_file(&props).unwrap();
            let made = std::process::Command::new("mkfifo").arg(&props).status();
            assert!(made.is_ok_and(|s| s.success()), "mkfifo");
            assert_eq!(run(&sub), 1);
        }
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
    fn removed_api() {
        let src = "apply plugin: 'maven'\napply(plugin: 'findbugs')\nplugins {\n    id 'osgi'\n    id 'java'\n}\njar {\n    classifier = 'all'\n    baseName 'x'\n    version = '1'\n}\ntask docs(type: Zip) {\n    appendix = 'docs'\n}\ntasks.register('src', Jar) { archiveName = 'a.jar' }\ntasks.withType(Tar).configureEach { extension = 'tgz' }\nsubprojects { war { destinationDir = file('d') } }\nuploadArchives {\n    repositories { }\n}\nfrom configurations.runtime\ndef cp = project.configurations.testCompile\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![
                ("GradleRemovedApi", 1),
                ("GradleRemovedApi", 2),
                ("GradleRemovedApi", 4),
                ("GradleRemovedApi", 8),
                ("GradleRemovedApi", 9),
                ("GradleRemovedApi", 13),
                ("GradleRemovedApi", 15),
                ("GradleRemovedApi", 16),
                ("GradleRemovedApi", 17),
                ("GradleRemovedApi", 18),
                ("GradleRemovedApi", 21),
                ("GradleRemovedApi", 22),
            ]
        );
        // not the same names elsewhere: a dependency's artifact classifier, a local
        // variable, a jar block that isn't the task, the new properties, a declared
        // configuration, a build's own uploadArchives task, an id outside plugins { }
        let src = "def archiveName = 'x'\ndependencies {\n    implementation('a:b:1') {\n        artifact {\n            classifier = 'jakarta'\n        }\n    }\n}\npublishing { publications { maven(MavenPublication) { artifact(sourcesJar) { classifier 'sources' } } } }\njar {\n    archiveClassifier = 'all'\n    if (GradleVersion.current() < GradleVersion.version('6.0')) {\n        baseName = 'old'\n    }\n}\nconfigurations { runtime }\nfrom configurations.runtime\ntask uploadArchives\nuploadArchives { }\ndependencies { id 'maven' }\napply plugin: 'maven-publish'\n";
        assert_eq!(rules("build.gradle", src), vec![]);
    }

    #[test]
    fn removed_api_follows_the_wrapper() {
        let root = std::env::temp_dir().join(format!("groovyfmt-removed-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let props = root.join("gradle/wrapper/gradle-wrapper.properties");
        let src = "apply plugin: 'findbugs'\napply plugin: 'maven'\njar { classifier = 'x' }\n";
        let run = |v: &str| {
            std::fs::write(&props, format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n")).unwrap();
            rules(root.join("build.gradle").to_str().unwrap(), src)
                .iter()
                .map(|r| r.1)
                .collect::<Vec<_>>()
        };
        // findbugs went in 6.0, maven in 7.0, classifier in 8.0
        assert_eq!(run("5.6"), vec![]);
        assert_eq!(run("6.9"), vec![1]);
        assert_eq!(run("7.6"), vec![1, 2]);
        assert_eq!(run("8.14"), vec![1, 2, 3]);
        std::fs::remove_dir_all(&root).unwrap();
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
