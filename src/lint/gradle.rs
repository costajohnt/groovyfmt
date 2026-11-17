//! Gradle build scripts (`*.gradle`): syntax that current Gradle versions removed.
//! Versions and replacements are from the upgrade guides:
//! https://docs.gradle.org/current/userguide/upgrading_version_6.html (compile, runtime,
//! testCompile, testRuntime removed in 7.0; jcenter() deprecated),
//! https://docs.gradle.org/current/userguide/upgrading_major_version_9.html (jcenter() removed
//! in 9.0), https://docs.gradle.org/current/userguide/upgrading_version_4.html (`<<` task
//! actions deprecated in 4.10, gone in 5.0), https://docs.gradle.org/current/userguide/upgrading_version_5.html
//! (findbugs, jdepend, osgi and announce plugins removed in 6.0),
//! https://docs.gradle.org/current/userguide/upgrading_version_7.html (archive task
//! properties removed in 8.0), https://docs.gradle.org/current/userguide/upgrading_version_8.html
//! (space assignment deprecated in 8.12, "Upgrading from 8.11 and earlier"),
//! https://docs.gradle.org/current/userguide/upgrading_version_9.html (multi-string
//! dependency notation deprecated in 9.1, "Upgrading from 9.0.0 and earlier").

use super::jenkins::{Call, call, statements};
use super::{Edit, Violation, at, constant_value, has_interpolation, own_lines};
use crate::text;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::Node;

/// Removed configuration, replacement. Each but `compile` has one replacement, which
/// `--fix` makes; `compile`'s depends on whether the library's API exposes the dependency,
/// so it is fixed only with `gradle_compile_replacement` set (`opt_in_compile`).
/// Plugins that make the removed configurations' replacements (runtimeOnly,
/// testImplementation, testRuntimeOnly, implementation): Java, the plugins that apply it,
/// and Android's.
/// A rename is fixed only in a file that applies one; a file whose plugins come from a parent
/// (`allprojects`, a convention plugin) may have none of the new names, so it's only reported.
const JVM_PLUGINS: &[&str] = &[
    "java",
    "java-library",
    "groovy",
    "scala",
    "application",
    "war",
    "java-gradle-plugin",
    "kotlin",
    "org.jetbrains.kotlin.jvm",
    // the Android Gradle plugin's module types make the same configurations (api too, but
    // `compile` -> `api` waits for java-library or com.android.library, `opt_in_compile`)
    "com.android.application",
    "com.android.library",
    "com.android.dynamic-feature",
    "com.android.test",
    // apply java (and groovy), per the Ratpack manual's Gradle chapter
    "io.ratpack.ratpack-java",
    "io.ratpack.ratpack-groovy",
];

/// The message when a rename isn't fixed for want of a plugin from `JVM_PLUGINS`.
const NO_JVM_PLUGIN: &str =
    " (not fixed: this file applies no Java plugin, so the new configuration may not exist)";

const REMOVED: &[(&str, &str)] = &[
    (
        "compile",
        "implementation (or api for dependencies your API exposes); --fix replaces it once `gradle_compile_replacement` is set under [lint]",
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
    let deprecated_space = std::cell::OnceCell::new();
    let mut plugins = HashSet::new();
    // the script's own methods: its `compile(x)` or `jcenter()` is no configuration (#192)
    let mut methods = HashSet::new();
    crate::preorder(root, |n| {
        plugins.extend(applied_plugin(src, n));
        if n.kind() == "method_declaration"
            && let Some(m) = n.child_by_field_name("name")
        {
            methods.insert(text(m, src));
        }
        removed_api(src, n, &mut removed, &mut conf_refs);
        if let Some(names) = property_scope(src, n)
            && !path
                .file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f == "settings.gradle" || f.ends_with("init.gradle"))
        {
            for s in statements(n) {
                space_assignment(src, s, names, &deprecated_space, path, v);
            }
        }
        match n.kind() {
            "method_invocation" => jcenter(src, n, v),
            "command_chain" => {
                jcenter(src, n, v);
                if n.child_by_field_name("receiver")
                    .is_some_and(|r| text(r, src) == "task")
                {
                    left_shift(src, n, v);
                }
            }
            "expression_statement" if text(n, src) == "task" => left_shift(src, n, v),
            _ => {}
        }
        if let Some(c) = call(n, src)
            && let Some(b) = c.body
            && c.name == "dependencies"
        {
            deps.push(b);
        }
        declares(src, n, &mut declared);
        true
    });
    // read only for a file that would get a report
    let old_gradle = std::cell::OnceCell::new();
    let applier = std::cell::OnceCell::new();
    let jvm = plugins.iter().any(|p| JVM_PLUGINS.contains(&p.as_str()));
    for b in deps {
        for s in statements(b) {
            let Some(c) = call(s, src) else { continue };
            if c.args.is_empty() {
                continue;
            }
            // `buildscript { dependencies { } }` only has `classpath`: a `compile` there was
            // never valid, and `implementation` isn't either
            if let Some((_, new)) = REMOVED.iter().find(|(old, _)| *old == c.name)
                && !declared.contains(c.name)
                && !methods.contains(c.name)
                && !owners(s, src).any(|o| o == "buildscript")
                && !*old_gradle.get_or_init(|| wrapper_major(path).is_some_and(|m| m < 7))
                && !applier
                    .get_or_init(|| applier_declared(path))
                    .contains(c.name)
            {
                // the name starts the statement: `testCompile 'a'`, `testCompile('a') { }`
                let fix =
                    (c.name != "compile" && jvm && text(s, src).starts_with(c.name)).then(|| {
                        Edit {
                            range: s.start_byte()..s.start_byte() + c.name.len(),
                            with: new.to_string(),
                        }
                    });
                let why = if c.name != "compile" && !jvm {
                    NO_JVM_PLUGIN
                } else {
                    ""
                };
                v.push(Violation {
                    fix,
                    ..at(
                        s,
                        "GradleRemovedConfiguration",
                        format!(
                            "The `{}` configuration was removed in Gradle 7.0; use {new}{why}",
                            c.name
                        ),
                    )
                });
            }
            dynamic_version(src, &c, v);
            if !NOT_CONFIGURATIONS.contains(&c.name) {
                parentheses(src, &c, v);
                tuple(src, &c, v);
            }
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
    if methods.contains("jcenter") {
        v.retain(|x| x.rule != "GradleJcenter");
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

/// Adds to `declared` the configurations `n` makes: the names a `configurations { }` block
/// (`n`) makes or reads, or `configurations.create('compile')` and the like.
fn declares(src: &str, n: Node, declared: &mut HashSet<String>) {
    if let Some(c) = call(n, src)
        && let Some(b) = c.body
        && c.name == "configurations"
    {
        for s in statements(b) {
            // create('runtime'), register('runtime') { }, ...
            if let Some(c) = call(s, src)
                && DECLARING.contains(&c.name)
                && let Some(name) = c.args.first().and_then(|a| constant_value(*a, src))
            {
                declared.insert(name);
            }
            let t = text(s, src);
            let end = t
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .unwrap_or(t.len());
            declared.insert(t[..end].to_string());
        }
        // a name the block reads makes it too, as Gradle runs it:
        // `integrationTestRuntime.extendsFrom testRuntime` (#145)
        crate::preorder(b, |x| {
            if x.kind() == "identifier"
                && x.parent()
                    .is_none_or(|p| p.child_by_field_name("field") != Some(x))
            {
                declared.insert(text(x, src).to_string());
            }
            true
        });
    }
    // configurations.create('compile'), maybeCreate, register, getByName, named
    if n.kind() == "method_invocation"
        && n.child_by_field_name("function").is_some_and(|f| {
            let t = text(f, src);
            DECLARING
                .iter()
                .any(|d| t.strip_suffix(d).is_some_and(|r| r.ends_with('.')))
        })
        && let Some(a) = n
            .child_by_field_name("arguments")
            .and_then(|a| a.named_child(0))
        && let Some(name) = constant_value(a, src)
    {
        declared.insert(name);
    }
}

/// The most bytes read of a script that may apply the one linted (`applier_declared`).
const MAX_SCRIPT_BYTES: u64 = 4 * 1024 * 1024;

/// The configurations made by the `.gradle` scripts that apply `path` with
/// `apply from: 'deps.gradle'`, as a build script declares `testCompile` and the
/// `dependencies.gradle` it applies uses it (#217). The scripts looked at are those in the
/// script's directory and the ones above it, up to the build's root (the nearest directory
/// with `settings.gradle`, `settings.gradle.kts` or `.git`; without one, only the script's
/// own directory). The target must be a plain path to `path` relative to the applying
/// script's directory: a URL, an interpolated name or a missing file applies nothing here.
fn applier_declared(path: &Path) -> HashSet<String> {
    let mut declared = HashSet::new();
    let Ok(me) = path.canonicalize() else {
        return declared;
    };
    let root = me.ancestors().skip(1).find(|d| {
        ["settings.gradle", "settings.gradle.kts", ".git"]
            .iter()
            .any(|f| d.join(f).exists())
    });
    for dir in me.ancestors().skip(1) {
        let scripts = std::fs::read_dir(dir).into_iter().flatten().flatten();
        for p in scripts.map(|e| e.path()) {
            if p.extension().is_none_or(|e| e != "gradle") || p == me {
                continue;
            }
            let Some(src) = read_small(&p, MAX_SCRIPT_BYTES) else {
                continue;
            };
            let tree = crate::parse(&src);
            let mut applies = false;
            crate::preorder(tree.root_node(), |n| {
                applies |= apply_argument(&src, n, "from")
                    .is_some_and(|t| dir.join(t).canonicalize().is_ok_and(|t| t == me));
                !applies
            });
            if applies {
                crate::preorder(tree.root_node(), |n| {
                    declares(&src, n, &mut declared);
                    true
                });
            }
        }
        if root.is_none_or(|r| r == dir) {
            break;
        }
    }
    declared
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
    // apply plugin: 'maven', apply(plugin: 'maven'), plugins { id 'maven' }
    if let Some(id) = applied_plugin(src, n)
        && let Some((_, since, new)) = REMOVED_PLUGINS.iter().find(|p| p.0 == id)
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

/// The plugin id `n` applies: `apply plugin: 'java'` or `apply(plugin: 'java')` (`n` is the
/// named argument), or `id 'java'` in a `plugins { }` block (`n` is the statement; a
/// `version` after the id is allowed).
fn applied_plugin(src: &str, n: Node) -> Option<String> {
    if n.kind() == "named_argument" {
        return apply_argument(src, n, "plugin");
    }
    let c = call(n, src)?;
    let a = c.args.first().filter(|a| {
        c.name == "id"
            && a.kind() == "string_literal"
            && (c.args.len() == 1 || (c.args.len() == 3 && text(c.args[1], src) == "version"))
    })?;
    if owners(n, src).next() != Some("plugins") {
        return None;
    }
    constant_value(*a, src)
}

/// The string the named argument `n` passes to `apply` as `key`: `apply plugin: 'java'`,
/// `apply(from: 'deps.gradle')`.
fn apply_argument(src: &str, n: Node, key: &str) -> Option<String> {
    if n.kind() != "named_argument" {
        return None;
    }
    let val = n.child_by_field_name("value")?;
    let applies = n
        .child_by_field_name("key")
        .is_some_and(|k| text(k, src) == key)
        && val.kind() == "string_literal"
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
            .is_some_and(|f| text(f, src) == "apply");
    applies.then(|| constant_value(val, src)).flatten()
}

/// Calls in a `configurations { }` block (or on `configurations.`) that make or name a
/// configuration, which the build then has.
const DECLARING: &[&str] = &[
    "create",
    "maybeCreate",
    "register",
    "getByName",
    "named",
    "findByName",
];

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
    wrapper_version(path).map(|v| v.0)
}

/// The nearest wrapper's Gradle version as (major, minor), as for `wrapper_major`.
fn wrapper_version(path: &Path) -> Option<(u32, u32)> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let props = dir
        .ancestors()
        .map(|d| d.join("gradle/wrapper/gradle-wrapper.properties"))
        .find(|p| p.is_file())?;
    version(&read_small(&props, MAX_WRAPPER_BYTES)?)
}

/// The real file is a few hundred bytes.
const MAX_WRAPPER_BYTES: u64 = 64 * 1024;

/// `p` if it is a regular file of at most `max` bytes, in UTF-8. A FIFO or device
/// is never opened (opening a FIFO waits for a writer); one swapped in between the check and
/// the open is the race the source reads accept too (see `write_in_place`).
fn read_small(p: &Path, max: u64) -> Option<String> {
    use std::io::Read;
    let ok = |m: std::fs::Metadata| m.is_file() && m.len() <= max;
    if !std::fs::metadata(p).is_ok_and(ok) {
        return None;
    }
    let f = std::fs::File::open(p).ok()?;
    if !f.metadata().is_ok_and(ok) {
        return None;
    }
    let mut bytes = Vec::new();
    f.take(max + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// `gradle-8.12.1-bin.zip` is (8, 12), `gradle-7.0-rc-1-bin.zip` (7, 0), `gradle-9-bin.zip` (9, 0).
fn version(props: &str) -> Option<(u32, u32)> {
    let url = props
        .lines()
        .find_map(|l| l.trim().strip_prefix("distributionUrl"))?;
    let mut parts = url.rsplit("gradle-").next()?.split(['.', '-']);
    let major = parts.next()?.parse().ok()?;
    Some((
        major,
        parts.next().and_then(|m| m.parse().ok()).unwrap_or(0),
    ))
}

/// `jcenter()` or `jcenter { ... }` in a repositories block. A plain `jcenter()` statement
/// in a `repositories { }` block is fixed: it becomes `mavenCentral()`, or its line goes
/// when the block already has a plain `mavenCentral()`.
fn jcenter(src: &str, n: Node, v: &mut Vec<Violation>) {
    let f = n
        .child_by_field_name("function")
        .or_else(|| n.child_by_field_name("receiver"));
    let Some(f) = f.filter(|f| matches!(text(*f, src), "jcenter" | "repositories.jcenter")) else {
        return;
    };
    let fix = n
        .parent()
        .filter(|s| {
            text(*s, src) == "jcenter()"
                && s.kind() == "expression_statement"
                && owners(*s, src).next() == Some("repositories")
        })
        .and_then(|s| {
            let block = s.parent()?;
            if statements(block)
                .iter()
                .any(|x| text(*x, src) == "mavenCentral()")
            {
                own_lines(src, s)
            } else {
                Some(Edit {
                    range: f.byte_range(),
                    with: "mavenCentral".into(),
                })
            }
        });
    v.push(Violation {
        fix,
        ..at(
            n,
            "GradleJcenter",
            "jcenter() was deprecated in Gradle 7.0 and removed in 9.0, and JCenter is read-only; use mavenCentral()".into(),
        )
    });
}

/// `task hello << { ... }`. The grammar reads it as two statements, `task` and
/// `hello << { ... }`, so this looks at the statement after a bare `task` on the same line;
/// `task 'hello' << { }` is one command whose argument is the `<<`.
fn left_shift(src: &str, n: Node, v: &mut Vec<Violation>) {
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
    let (Some(op), Some(body)) = (
        e.child_by_field_name("operator"),
        e.child_by_field_name("right"),
    ) else {
        return;
    };
    if e.kind() != "binary_expression" || op.kind() != "<<" || body.kind() != "closure" {
        return;
    }
    // `task hello << { }` and `task copy(type: Copy) << { }` become
    // `task hello { doLast { } }`; not the quoted form, whose block form differs
    let named = e.child_by_field_name("left").is_some_and(|l| {
        l.kind() == "identifier"
            || (l.kind() == "method_invocation"
                && l.child_by_field_name("closure").is_none()
                && l.child_by_field_name("function")
                    .is_some_and(|f| f.kind() == "identifier"))
    });
    // only spaces between `<<` and `{`, and `task name <<` on one line: anything else (a
    // comment, a line break) would be lost or moved
    let plain = !src[n.start_byte()..op.start_byte()].contains('\n')
        && src[op.end_byte()..body.start_byte()]
            .chars()
            .all(|ch| ch == ' ' || ch == '\t');
    let fix = (n.kind() == "expression_statement" && named && plain).then(|| Edit {
        range: n.start_byte()..body.end_byte(),
        with: format!(
            "{}{{ doLast {} }}",
            &src[n.start_byte()..op.start_byte()],
            text(body, src)
        ),
    });
    v.push(Violation {
        fix,
        ..at(
            n,
            "GradleLeftShiftTask",
            "`task name << { }` was removed in Gradle 5.0; write task name { doLast { ... } }"
                .into(),
        )
    });
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

/// Calls in a dependencies block that aren't configurations.
const NOT_CONFIGURATIONS: &[&str] = &[
    "add",
    "artifactTypes",
    "attributesSchema",
    "components",
    "constraints",
    "create",
    "enforcedPlatform",
    "files",
    "fileTree",
    "gradleApi",
    "gradleTestKit",
    "localGroovy",
    "module",
    "modules",
    "platform",
    "project",
    "registerTransform",
    "testFixtures",
    "variantOf",
];

/// Whether only `,` and whitespace separate `args`, so rewriting the text around them
/// drops no comment.
fn plain_list(src: &str, args: &[Node]) -> bool {
    args.windows(2)
        .all(|w| src[w[0].end_byte()..w[1].start_byte()].trim() == ",")
}

/// GradleDependencyParentheses: `implementation('a:b:1')` with no closure after it. The
/// arguments must read the same as a command's: a string, a name, a dotted name, a call
/// without a closure or a named argument, all on one line, each starting with a letter, `_`
/// or a quote. Anything else first would read differently without the parentheses: `[`
/// as an index (`implementation [0, 1].first()`), `(` as the call's own arguments, `-` as a
/// subtraction, `/` as a division. `fix::check` also parses the result to be sure.
fn parentheses(src: &str, c: &Call, v: &mut Vec<Violation>) {
    let Some(e) = c.node.named_child(0).filter(|e| {
        c.node.kind() == "expression_statement"
            && e.kind() == "method_invocation"
            && e.child_by_field_name("closure").is_none()
    }) else {
        return;
    };
    let Some(list) = e.child_by_field_name("arguments") else {
        return;
    };
    let (Some(first), Some(last)) = (c.args.first(), c.args.last()) else {
        return;
    };
    let ok = list.named_child_count() == c.args.len()
        && c.args.iter().all(|a| match a.kind() {
            "string_literal" | "identifier" | "field_access" | "named_argument" => true,
            "method_invocation" => a.child_by_field_name("closure").is_none(),
            _ => false,
        } && text(*a, src)
            .starts_with(|ch: char| ch.is_ascii_alphabetic() || matches!(ch, '_' | '\'' | '"')))
        && plain_list(src, &c.args)
        && src[list.start_byte() + 1..first.start_byte()]
            .trim()
            .is_empty()
        && src[last.end_byte()..list.end_byte() - 1].trim().is_empty()
        && !text(list, src).contains('\n');
    if !ok {
        return;
    }
    let args = &src[first.start_byte()..last.end_byte()];
    let mut x = at(
        c.node,
        "GradleDependencyParentheses",
        "Unnecessary parentheses around the dependency; write it without them".into(),
    );
    x.fix = Some(Edit {
        range: c.node.byte_range(),
        with: format!("{} {args}", c.name),
    });
    v.push(x);
}

/// GradleDependencyTuple: `implementation group: 'a', name: 'b', version: '1'`, which the
/// string `'a:b:1'` says the same as. Only `group`, `name` and `version` keys (group and
/// name required), each a plain string with no `:`, `@`, quote, `\`, `$` or space.
fn tuple(src: &str, c: &Call, v: &mut Vec<Violation>) {
    let (Some(first), Some(last)) = (c.args.first(), c.args.last()) else {
        return;
    };
    let mut parts: [Option<String>; 3] = Default::default();
    let mut double = true;
    for a in &c.args {
        let (Some(k), Some(val)) = (
            a.child_by_field_name("key")
                .filter(|_| a.kind() == "named_argument"),
            a.child_by_field_name("value"),
        ) else {
            return;
        };
        let Some(i) = constant_value(k, src)
            .and_then(|k| ["group", "name", "version"].iter().position(|x| *x == k))
        else {
            return;
        };
        let t = text(val, src);
        let Some(s) = constant_value(val, src).filter(|s| {
            val.kind() == "string_literal"
                && !t.starts_with("'''")
                && !t.starts_with("\"\"\"")
                && !s.is_empty()
                && !s.contains(|ch: char| ":@'\"\\$".contains(ch) || ch.is_whitespace())
        }) else {
            return;
        };
        if parts[i].replace(s).is_some() {
            return;
        }
        double &= t.starts_with('"');
    }
    let [Some(g), Some(n), ver] = &parts else {
        return;
    };
    if !plain_list(src, &c.args) {
        return;
    }
    let q = if double { '"' } else { '\'' };
    let notation = match ver {
        Some(ver) => format!("{q}{g}:{n}:{ver}{q}"),
        None => format!("{q}{g}:{n}{q}"),
    };
    let s = c.node;
    let mut x = at(
        s,
        "GradleDependencyTuple",
        format!("Dependency in map notation (deprecated in Gradle 9.1); write {notation}"),
    );
    x.fix = Some(Edit {
        range: s.byte_range(),
        with: format!(
            "{}{notation}{}",
            &src[s.start_byte()..first.start_byte()],
            &src[last.end_byte()..s.end_byte()]
        ),
    });
    v.push(x);
}

const PROJECT_PROPERTIES: &[&str] = &["description", "group", "version"];
const TASK_PROPERTIES: &[&str] = &["description", "group"];

/// The properties whose space assignment GradleSpaceAssignment reports among the statements
/// of `n`: `description`, `group` and `version` at the top level of the script and in an
/// `allprojects`, `subprojects` or `project(':x')` block, where they set the project's;
/// `description` and `group` in a task's block (`task x { }`, `task x(type: T) { }`,
/// `task('x') { }`, `tasks.register('x') { }`, `tasks.create` and `tasks.named`).
fn property_scope(src: &str, n: Node) -> Option<&'static [&'static str]> {
    if n.kind() == "source_file" {
        return Some(PROJECT_PROPERTIES);
    }
    if n.kind() != "closure" {
        return None;
    }
    let p = n.parent()?;
    // `task x { }` reads as `task` and then `x { }`
    let after_task = |stmt: Node| {
        stmt.prev_named_sibling().is_some_and(|t| {
            t.kind() == "expression_statement"
                && text(t, src) == "task"
                && t.end_position().row == stmt.start_position().row
        })
    };
    let name = match p.kind() {
        "command_chain" => {
            let c = call(p, src)?;
            if c.body != Some(n) || !c.args.is_empty() {
                return None;
            }
            if after_task(p) {
                return Some(TASK_PROPERTIES);
            }
            c.name
        }
        "method_invocation" if p.child_by_field_name("closure") == Some(n) => {
            let f = p.child_by_field_name("function")?;
            let t = text(f, src);
            if f.kind() == "field_access" {
                let tail: Vec<&str> = t.rsplit('.').take(2).collect();
                return (tail.len() == 2
                    && tail[1] == "tasks"
                    && matches!(tail[0], "register" | "create" | "named"))
                .then_some(TASK_PROPERTIES);
            }
            if p.parent().is_some_and(after_task) || t == "task" {
                return Some(TASK_PROPERTIES);
            }
            if t == "project" {
                return Some(PROJECT_PROPERTIES);
            }
            t
        }
        _ => return None,
    };
    matches!(name, "allprojects" | "subprojects").then_some(PROJECT_PROPERTIES)
}

/// GradleSpaceAssignment: `description 'x'` for `description = 'x'`, a property in `names`
/// set with one argument and no parentheses, alone on its line but for `;`, braces and a
/// comment: `group 'a' version 'b'` is the call chain `group('a').version('b')`. Quiet under a wrapper pinning a Gradle before
/// 8.12, which deprecated it.
fn space_assignment(
    src: &str,
    s: Node,
    names: &[&str],
    deprecated: &std::cell::OnceCell<bool>,
    path: &Path,
    v: &mut Vec<Violation>,
) {
    let Some(c) = call(s, src) else { return };
    let [arg] = c.args.as_slice() else { return };
    let command = s.kind() == "command_chain"
        || (s.kind() == "expression_statement"
            && s.named_child(0).is_some_and(|e| e.kind() == "identifier"));
    let simple = match arg.kind() {
        "string_literal" | "number_literal" | "identifier" | "field_access"
        | "binary_expression" => true,
        "method_invocation" => arg.child_by_field_name("closure").is_none(),
        _ => false,
    };
    if !command
        || !simple
        || c.body.is_some()
        || !names.contains(&c.name)
        || !text(s, src).starts_with(c.name)
        || !src[s.start_byte() + c.name.len()..arg.start_byte()]
            .trim()
            .is_empty()
        || !alone(src, s.start_byte(), arg.end_byte())
        || !*deprecated.get_or_init(|| wrapper_version(path).is_none_or(|v| v >= (8, 12)))
        || shadowed(src, s, c.name)
    {
        return;
    }
    let name = c.name;
    let mut x = at(
        s,
        "GradleSpaceAssignment",
        format!(
            "`{name} value` assignment was deprecated in Gradle 8.12 and stops working in Gradle 10; write {name} = value"
        ),
    );
    let e = arg.end_position();
    (x.end_line, x.end_col) = (e.row + 1, e.column + 1);
    x.span = s.start_byte()..arg.end_byte();
    x.fix = Some(Edit {
        range: x.span.clone(),
        with: format!("{name} = {}", text(*arg, src)),
    });
    v.push(x);
}

/// Whether a block around `s` declares a variable `name` (`def version = '1'`, at any
/// place in the block): then `version x` calls that variable, and `version = x` would
/// assign it instead of the project's property (#192).
fn shadowed(src: &str, s: Node, name: &str) -> bool {
    std::iter::successors(s.parent(), |p| p.parent()).any(|b| {
        statements(b).iter().any(|d| {
            matches!(
                d.kind(),
                "local_variable_declaration" | "field_declaration" | "multi_assignment_declaration"
            ) && super::unused::declarators(src, *d)
                .iter()
                .any(|(n, _)| *n == name)
        })
    })
}

/// The statement `src[start..end]` is one of its own, not part of a call chain
/// (`group 'a' version 'b'`, `version 'a' call()`, a `\` continuation): on its line only
/// a block's `{` or a `;` comes before it, with no `\` ending the line above, and only `;`,
/// a block's `}` or a `//` comment after it.
fn alone(src: &str, start: usize, end: usize) -> bool {
    let before = src[..start].rsplit('\n').next().unwrap_or("").trim_end();
    let after = src[end..].lines().next().unwrap_or("").trim_start();
    (before.is_empty() || before.ends_with(['{', ';']))
        && !src[..start].trim_end().ends_with('\\')
        && (after.is_empty() || after.starts_with([';', '}']) || after.starts_with("//"))
}

/// GradleRemovedConfiguration on `compile` names the replacement the config gives,
/// `implementation` or `api` (`gradle_compile_replacement`), and gets a fix when the file
/// applies a plugin that makes it: `api` only comes with java-library.
pub(super) fn opt_in_compile(vs: &mut [Violation], src: &str, with: &str) {
    let mut plugins = None;
    for x in vs {
        let rest = &src[x.span.start..];
        if x.rule != "GradleRemovedConfiguration"
            || x.fix.is_some()
            || !rest.starts_with("compile")
            || rest[7..].starts_with(|ch: char| ch.is_alphanumeric() || ch == '_')
        {
            continue;
        }
        let plugins = plugins.get_or_insert_with(|| {
            let tree = crate::parse(src);
            let mut ps = HashSet::new();
            crate::preorder(tree.root_node(), |n| {
                ps.extend(applied_plugin(src, n));
                true
            });
            ps
        });
        let why = if !plugins.iter().any(|p| JVM_PLUGINS.contains(&p.as_str())) {
            NO_JVM_PLUGIN
        } else if with == "api"
            && !plugins.contains("java-library")
            && !plugins.contains("com.android.library")
        {
            " (not fixed: api needs the java-library or com.android.library plugin, which this file doesn't apply)"
        } else {
            x.fix = Some(Edit {
                range: x.span.start..x.span.start + 7,
                with: with.to_string(),
            });
            ""
        };
        x.message = format!(
            "The `compile` configuration was removed in Gradle 7.0; use {with} (`gradle_compile_replacement`){why}"
        );
    }
}

/// The call statement spanning exactly `r` (with an argument the grammar reads as the next
/// statement, `api project(':x')`): its name, its arguments' kinds and texts, and its
/// closure's text.
type CallText<'a> = (&'a str, Vec<(&'a str, &'a str)>, Option<&'a str>);

fn call_at<'a>(src: &'a str, root: Node<'a>, r: &std::ops::Range<usize>) -> Option<CallText<'a>> {
    let n = root.descendant_for_byte_range(r.start, r.start + 1)?;
    std::iter::successors(Some(n), |p| p.parent())
        .take_while(|p| p.start_byte() == r.start && p.end_byte() <= r.end)
        .filter_map(|p| call(p, src))
        .find(|c| {
            let ends = c.args.iter().chain(&c.body).map(|a| a.end_byte());
            ends.chain([c.node.end_byte()]).max() == Some(r.end)
        })
        .map(|c| {
            let args = c.args.iter().map(|a| (a.kind(), text(*a, src))).collect();
            (c.name, args, c.body.map(|b| text(b, src)))
        })
}

/// For `fix::check`, on a rewrite of a dependency (GradleDependencyParentheses,
/// GradleDependencyTuple): `before[range]` of `src` and `after[ins]` of `out` parse as the
/// same call, with the same name, closure and arguments, or with map notation become one
/// string. Without the parentheses, `implementation [0, 1].first()` indexes a property and
/// `implementation ('x').toUpperCase()` calls a method on the result.
pub(crate) fn same_call(
    src: &str,
    before: Node,
    range: &std::ops::Range<usize>,
    out: &str,
    after: Node,
    ins: &std::ops::Range<usize>,
) -> bool {
    let (Some(old), Some(new)) = (call_at(src, before, range), call_at(out, after, ins)) else {
        return false;
    };
    old.0 == new.0
        && old.2 == new.2
        && (old.1 == new.1
            || (old.1.iter().all(|a| a.0 == "named_argument")
                && matches!(new.1.as_slice(), [("string_literal", _)])))
}

/// For `fix::check`, on a GradleSpaceAssignment rewrite: `before[range]` of `src` is a call
/// of one argument, and `after[ins]` of `out` a statement of its own that assigns that
/// argument's text to the property the call named.
pub(crate) fn same_assignment(
    src: &str,
    before: Node,
    range: &std::ops::Range<usize>,
    out: &str,
    after: Node,
    ins: &std::ops::Range<usize>,
) -> bool {
    let Some((name, args, None)) = call_at(src, before, range) else {
        return false;
    };
    let [(_, value)] = args.as_slice() else {
        return false;
    };
    let Some(n) = after.descendant_for_byte_range(ins.start, ins.start + 1) else {
        return false;
    };
    std::iter::successors(Some(n), |p| p.parent())
        .take_while(|p| p.start_byte() == ins.start && p.end_byte() <= ins.end)
        .any(|p| {
            p.kind() == "assignment_expression"
                && p.end_byte() == ins.end
                && p.parent()
                    .is_some_and(|s| s.kind() == "expression_statement")
                && p.child_by_field_name("left")
                    .is_some_and(|l| l.kind() == "identifier" && text(l, out) == name)
                && p.child_by_field_name("right")
                    .is_some_and(|r| text(r, out) == *value)
        })
        && alone(out, ins.start, ins.end)
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
                ("GradleDependencyTuple", 5),
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
    fn removed_configurations_declared_by_an_applying_script() {
        // build.gradle declares testCompile and applies deps.gradle, which uses it (#217)
        let root = std::env::temp_dir().join(format!("groovyfmt-applied-{}", std::process::id()));
        let sub = root.join("gradle");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("settings.gradle"), "").unwrap();
        let uses = "dependencies {\n    testCompile 'a:b:1'\n}\n";
        let run = |p: &Path| {
            std::fs::write(p, uses).unwrap();
            rules(p.to_str().unwrap(), uses).len()
        };
        let build = |s: &str| std::fs::write(root.join("build.gradle"), s).unwrap();
        build(
            "configurations { testCompile }\napply from: 'deps.gradle'\napply(from: 'gradle/sub.gradle')\n",
        );
        // declared in the applying script, also one directory up
        assert_eq!(
            (run(&root.join("deps.gradle")), run(&sub.join("sub.gradle"))),
            (0, 0)
        );
        // a script nothing applies, a URL, an interpolated name: reported as before
        assert_eq!(run(&root.join("other.gradle")), 1);
        build(
            "configurations { testCompile }\napply from: 'https://x/deps.gradle'\napply from: \"${d}.gradle\"\n",
        );
        assert_eq!(run(&root.join("deps.gradle")), 1);
        // applied but declared nowhere
        build("apply from: 'deps.gradle'\n");
        assert_eq!(run(&root.join("deps.gradle")), 1);
        // the applying script names a missing file
        build("configurations { testCompile }\napply from: 'gone.gradle'\n");
        assert_eq!(run(&root.join("deps.gradle")), 1);
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
    fn removed_api_skips_a_configuration_the_file_makes() {
        // `configurations { testRuntime }` or reading the name in that block makes it (#145)
        for decl in [
            "configurations { testRuntime }\n",
            "configurations {\n    integrationTestRuntime.extendsFrom testRuntime\n}\n",
            "configurations {\n    foo.extendsFrom(testRuntime)\n}\n",
        ] {
            let src = format!(
                "{decl}configurations {{\n    it2.extendsFrom configurations.testRuntime\n}}\ndependencies {{\n    testRuntime 'a:b:1'\n}}\n"
            );
            assert_eq!(rules("build.gradle", &src), vec![], "{decl}");
        }
        // only the name it makes: `configurations.testRuntime` there, or another name, isn't one
        let src = "configurations {\n    a.extendsFrom configurations.testRuntime\n    b.extendsFrom runtime\n}\nx = configurations.testRuntime\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![("GradleRemovedApi", 2), ("GradleRemovedApi", 5)]
        );
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
                ("GradleDependencyTuple", 4),
                ("GradleDynamicVersion", 4),
            ]
        );
    }

    #[test]
    fn space_assignment() {
        let src = "group 'org.example'\nversion \"1.${x}\"\ndescription project.name\nversion versionName\nallprojects {\n    group 'a'\n}\nsubprojects { version '1' }\nproject(':a') { description 'x' }\ntask docs {\n    description 'x'\n    group 'y'\n}\ntask copy(type: Copy) { group 'y' }\ntask('t') { description 'x' }\ntasks.register('r') { group 'y' }\ntasks.named('test') { description 'x' }\n";
        let lines: Vec<usize> = rules("build.gradle", src).iter().map(|r| r.1).collect();
        assert_eq!(lines, vec![1, 2, 3, 4, 6, 8, 9, 11, 12, 14, 15, 16, 17]);
        // method calls and assignments, properties elsewhere, a task's `version` (the
        // project's, through the task), other shapes of argument
        let src = "apply plugin: 'java'\nplugins {\n    id 'x' version '1'\n}\ngroup = 'a'\nversion('1')\ndescription 'a', 'b'\nversion { '1' }\nversion ['1']\ndependencies {\n    implementation 'a:b:1'\n    version '1'\n}\npublishing { publications { maven(MavenPublication) { version '1' } } }\njar { description 'x' }\ntask docs { version '1' }\next { group 'x' }\nbuildscript { group 'x' }\nmavenCentral()\ndescription /* why */ 'x'\nnot.description 'x'\n";
        assert_eq!(rules("build.gradle", src), vec![]);
        // a settings or init script has no project
        assert_eq!(rules("settings.gradle", "version '1'\n"), vec![]);
        assert_eq!(rules("x.init.gradle", "version '1'\n"), vec![]);
    }

    #[test]
    fn script_names_are_not_gradle_ones() {
        // #192: with a variable of the name in scope, `version version` calls the variable
        for src in [
            "def version = '1.0'\nversion version\n",
            "def group = 'x'\ngroup 'y'\n",
            "task foo {\n    def description = 'a'\n    description 'b'\n}\n",
            "subprojects {\n    def version = '0'\n    version '1'\n}\n",
            "String version = '0'\nversion '1'\n",
            "import groovy.transform.Field\n@Field String version = '0'\nversion '1'\n",
            "def group = 'g0'\nallprojects {\n    group 'g'\n}\n",
            "def (version, x) = ['0', 1]\nversion '1'\n",
        ] {
            assert_eq!(rules("build.gradle", src), vec![], "{src}");
            assert_eq!(fixed("build.gradle", None, src), src, "{src}");
        }
        // a variable elsewhere doesn't hide the project's
        let src = "task x {\n    def version = 1\n}\nversion '1'\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![("GradleSpaceAssignment", 4)]
        );
        // `compile` in buildscript, or the script's own `compile`, `jcenter`
        for src in [
            "buildscript {\n    dependencies {\n        classpath 'a:b:1'\n        compile 'x:y:1'\n    }\n}\napply plugin: 'java'\n",
            "apply plugin: 'java'\ndef compile(x) { println x }\ndependencies {\n    compile 'a:b:1'\n}\n",
            "def jcenter() { 'x' }\nprintln jcenter()\n",
        ] {
            assert_eq!(rules("build.gradle", src), vec![], "{src}");
            assert_eq!(
                fixed("build.gradle", Some("implementation"), src),
                src,
                "{src}"
            );
        }
        assert_eq!(
            fixed(
                "build.gradle",
                Some("implementation"),
                "apply plugin: 'java'\ndependencies {\n    compile 'a:b:1'\n}\n"
            ),
            "apply plugin: 'java'\ndependencies {\n    implementation 'a:b:1'\n}\n"
        );
    }

    #[test]
    fn space_assignment_is_fixed_only_as_a_statement_of_its_own() {
        // Groovy reads these as call chains, `group('a').version('b')`,
        // `version('a').call()`, or a `\` continuation joins the next line on; the last two
        // aren't commands at all
        for src in [
            "group 'a' version 'b'\n",
            "version 'a' call()\n",
            "version 'a' call\n",
            "version x y\n",
            "version 'a' /* c */ group 'b'\n",
            "group 'a' \\\n  version 'b'\n",
            "version -1\n",
            "version [1]\n",
        ] {
            assert_eq!(rules("build.gradle", src), vec![], "{src}");
            assert_eq!(fixed("build.gradle", None, src), src, "{src}");
        }
        // one value, which the assignment gives the property just as the call did
        for (src, out) in [
            ("version 'a' + 'b'\n", "version = 'a' + 'b'\n"),
            ("version \"${x}\"\n", "version = \"${x}\"\n"),
            (
                "description \'\'\'multi\nline\'\'\'\n",
                "description = \'\'\'multi\nline\'\'\'\n",
            ),
            ("version 'a' // why\n", "version = 'a' // why\n"),
            // (the `;` goes as unnecessary)
            ("version 'a';\n", "version = 'a'\n"),
            (
                "version 'α😀' ; group 'ü'\n",
                "version = 'α😀' ; group = 'ü'\n",
            ),
            (
                "subprojects { version '1' }\n",
                "subprojects { version = '1' }\n",
            ),
            // a method call binds to the value: `version('x'.toString())`
            ("version 'x' .toString()\n", "version = 'x' .toString()\n"),
            ("version 'a'\n  .trim()\n", "version = 'a'\n  .trim()\n"),
            (
                "description 'x' \\\n  + 'y'\n",
                "description = 'x' \\\n  + 'y'\n",
            ),
            ("version x\n", "version = x\n"),
        ] {
            assert_eq!(fixed("build.gradle", None, src), out, "{src}");
        }
    }

    #[test]
    fn space_assignment_follows_the_wrapper() {
        let root = std::env::temp_dir().join(format!("groovyfmt-space-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let run = |v: &str| {
            std::fs::write(
                root.join("gradle/wrapper/gradle-wrapper.properties"),
                format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n"),
            )
            .unwrap();
            rules(root.join("build.gradle").to_str().unwrap(), "version '1'\n").len()
        };
        // deprecated in 8.12
        assert_eq!(
            (
                run("7.6"),
                run("8.11.1"),
                run("8.12"),
                run("8.12-rc-1"),
                run("9.2.0")
            ),
            (0, 0, 1, 1, 1)
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dependency_notation() {
        let src = "dependencies {\n    implementation('a:b:1')\n    api(libs.foo, project(':x'))\n    implementation(group: 'a', name: 'b')\n    implementation group: 'a', name: 'b', version: '1'\n    runtimeOnly(group: \"a\", name: \"b\", version: \"1\") { transitive = false }\n}\nbuildscript { dependencies { classpath('a:b:1') } }\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![
                ("GradleDependencyParentheses", 2),
                ("GradleDependencyParentheses", 3),
                ("GradleDependencyParentheses", 4),
                ("GradleDependencyTuple", 4),
                ("GradleDependencyTuple", 5),
                ("GradleDependencyTuple", 6),
                ("GradleDependencyParentheses", 8),
            ]
        );
        // a closure after it, a list or a negative first, several lines, a comment, a call
        // that isn't a configuration; map keys the string can't carry here, values that
        // aren't plain strings, a missing name
        let src = "dependencies {\n    implementation('a:b:1') { exclude group: 'c' }\n    implementation([group: 'a', name: 'b'])\n    implementation(\n        'a:b:1'\n    )\n    implementation('a:b:1' /* why */)\n    add('implementation', 'a:b:1')\n    implementation(platform('a:b:1')) { }\n    implementation group: 'a', name: 'b', version: '1', classifier: 'x'\n    implementation group: 'a', name: 'b', ext: 'zip'\n    implementation group: 'a', name: 'b', version: v\n    implementation group: 'a', name: 'b', version: \"${v}\"\n    implementation group: 'a', version: '1'\n    implementation group: 'a', name: 'b:c'\n    implementation name: 'b', group: 'a', name: 'c'\n}\nfoo('a:b:1')\n";
        assert_eq!(rules("build.gradle", src), vec![]);
    }

    /// `src` fixed with the rules on by default (and the off-by-default ones with `all`),
    /// checking that fixing it again changes nothing.
    fn fixed_with(path: &str, compile: Option<&str>, all: bool, src: &str) -> String {
        let wanted = |v: &crate::lint::Violation, _: &str| {
            all || crate::lint::RULES
                .iter()
                .any(|r| r.name == v.rule && r.default)
        };
        match crate::fix::fix(Path::new(path), src, compile, wanted) {
            crate::fix::Fix::Fixed(t, _) => {
                assert_eq!(
                    crate::fix::fix(Path::new(path), &t, compile, wanted),
                    crate::fix::Fix::Nothing,
                    "idempotent: {t}"
                );
                t
            }
            crate::fix::Fix::Nothing => src.to_string(),
            other => panic!("{src}: {other:?}"),
        }
    }

    fn fixed(path: &str, compile: Option<&str>, src: &str) -> String {
        fixed_with(path, compile, false, src)
    }

    #[test]
    fn fixes_several_reports_at_once() {
        let src = "apply plugin: 'java-library'\ngroup 'a'\nrepositories {\n    jcenter()\n}\nbuildscript {\n    repositories {\n        jcenter()\n        mavenCentral()\n    }\n    dependencies {\n        classpath group: 'a', name: 'b', version: '1'\n    }\n}\ndependencies {\n    testCompile('junit:junit:4.12')\n    runtime group: 'a', name: 'b', version: '1.+'\n    testRuntime project(':x')\n    compile 'a:b:1'\n}\ntask hello << {\n    println 'hi'\n}\ntask copy(type: Copy) << { }\ntask 'quoted' << { }\ntask docs {\n    description 'x'\n}\n";
        let out = fixed("build.gradle", None, src);
        assert_eq!(
            out,
            "apply plugin: 'java-library'\ngroup = 'a'\nrepositories {\n    mavenCentral()\n}\nbuildscript {\n    repositories {\n        mavenCentral()\n    }\n    dependencies {\n        classpath group: 'a', name: 'b', version: '1'\n    }\n}\ndependencies {\n    testImplementation('junit:junit:4.12')\n    runtimeOnly group: 'a', name: 'b', version: '1.+'\n    testRuntimeOnly project(':x')\n    compile 'a:b:1'\n}\ntask hello { doLast {\n    println 'hi'\n} }\ntask copy(type: Copy) { doLast { } }\ntask 'quoted' << { }\ntask docs {\n    description = 'x'\n}\n"
        );
        // of the rules on by default, what's left: `compile` (no replacement configured) and
        // the quoted task
        let left: Vec<_> = rules("build.gradle", &out)
            .into_iter()
            .map(|r| r.0)
            .filter(|r| crate::lint::RULES.iter().any(|x| x.name == *r && x.default))
            .collect();
        assert_eq!(
            left,
            vec!["GradleRemovedConfiguration", "GradleLeftShiftTask"]
        );
        // the style rules too, and `compile` once the config names its replacement
        let out = fixed_with("build.gradle", Some("api"), true, src);
        assert!(out.contains("classpath 'a:b:1'\n"), "{out}");
        assert!(
            out.contains("testImplementation 'junit:junit:4.12'\n"),
            "{out}"
        );
        assert!(out.contains("runtimeOnly 'a:b:1.+'\n"), "{out}");
        assert!(out.contains("    api 'a:b:1'\n"), "{out}");
    }

    #[test]
    fn fixes_only_where_the_report_fires() {
        // a configuration the build makes itself, a removed name outside dependencies
        let src = "configurations { compile; testCompile }\ndependencies {\n    compile 'a:b:1'\n    testCompile 'a:b:1'\n}\ntask compile { }\n";
        assert_eq!(fixed("build.gradle", Some("implementation"), src), src);
        // jcenter with a configuration block, or sharing its line, stays
        let src = "repositories {\n    jcenter { url 'x' }\n}\nrepositories { jcenter(); mavenCentral() }\n";
        assert_eq!(fixed("build.gradle", None, src), src);
        // a name that only starts with `compile`
        let src = "configurations { compileX }\ndependencies {\n    compileX 'a:b:1'\n}\n";
        assert_eq!(fixed("build.gradle", Some("api"), src), src);
        // not a Gradle file
        let src = "dependencies {\n    testCompile 'a:b:1'\n}\n";
        assert_eq!(fixed("Build.groovy", None, src), src);
        // under a wrapper pinning Gradle 6, which still has the configurations
        let root = std::env::temp_dir().join(format!("groovyfmt-fix6-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        std::fs::write(
            root.join("gradle/wrapper/gradle-wrapper.properties"),
            "distributionUrl=https\\://services.gradle.org/distributions/gradle-6.9-bin.zip\n",
        )
        .unwrap();
        let p = root.join("build.gradle");
        assert_eq!(fixed(p.to_str().unwrap(), Some("api"), src), src);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fixed_dependency_notation_parses_as_the_same_call() {
        use crate::lint::jenkins::call;
        let src = "apply plugin: 'java'\ndependencies {\n    implementation('a:b:1', libs.foo)\n    api(project(':x'))\n    runtimeOnly(name: 'b', group: 'a') { transitive = false }\n}\n";
        let out = fixed_with("build.gradle", None, true, src);
        assert_eq!(
            out,
            "apply plugin: 'java'\ndependencies {\n    implementation 'a:b:1', libs.foo\n    api project(':x')\n    runtimeOnly('a:b') { transitive = false }\n}\n"
        );
        let tree = crate::parse(&out);
        let body = tree
            .root_node()
            .named_child(1)
            .and_then(|d| call(d, &out))
            .and_then(|c| c.body)
            .unwrap();
        let calls: Vec<(String, Vec<String>)> = super::statements(body)
            .into_iter()
            .filter_map(|s| call(s, &out))
            .map(|c| {
                let args = c
                    .args
                    .iter()
                    .map(|a| crate::text(*a, &out).to_string())
                    .collect();
                (c.name.to_string(), args)
            })
            .collect();
        assert_eq!(
            calls,
            vec![
                (
                    "implementation".into(),
                    vec!["'a:b:1'".into(), "libs.foo".into()]
                ),
                ("api".into(), vec!["project(':x')".into()]),
                ("runtimeOnly".into(), vec!["'a:b'".into()]),
            ]
        );
    }

    #[test]
    fn parentheses_whose_removal_changes_the_call_are_not_reported() {
        // without the parentheses: a property indexed, a method called on the result, a
        // division, a dollar-slashy string's `$`
        for arg in [
            "[0, 1].first()",
            "('x').toUpperCase()",
            "['a:b:1'].size",
            "/a:b:1/",
            "$/a:b:1/$",
            "-x",
            "(x)",
            "!x",
            "~/a/",
            "*xs",
        ] {
            let src = format!(
                "apply plugin: 'java'\ndependencies {{\n    implementation({arg})\n    testCompile 'a:b:1'\n}}\n"
            );
            assert_eq!(
                rules("build.gradle", &src),
                vec![("GradleRemovedConfiguration", 4)],
                "{arg}"
            );
            // the line after is still fixed
            assert_eq!(
                fixed_with("build.gradle", None, true, &src),
                src.replace("testCompile", "testImplementation"),
                "{arg}"
            );
        }
    }

    #[test]
    fn configurations_the_build_makes_are_not_renamed() {
        for decl in [
            "configurations {\n    create(\"runtime\")\n    register(\"testRuntime\")\n}\n",
            "configurations {\n    maybeCreate 'runtime'\n    getByName('testRuntime') { }\n}\n",
            "configurations {\n    named('runtime')\n    findByName('testRuntime')\n}\n",
            "configurations {\n    runtime\n    testRuntime { transitive = false }\n}\n",
            "configurations {\n    runtime.extendsFrom x\n    y.extendsFrom testRuntime\n}\n",
            "configurations.register('runtime')\nconfigurations.getByName('testRuntime')\n",
        ] {
            let src = format!(
                "apply plugin: 'java'\n{decl}dependencies {{\n    runtime \"a:b:1\"\n    testRuntime \"a:b:1\"\n}}\n"
            );
            assert_eq!(rules("build.gradle", &src), vec![], "{decl}");
            assert_eq!(fixed("build.gradle", None, &src), src, "{decl}");
        }
    }

    #[test]
    fn renames_are_fixed_only_under_a_java_plugin() {
        let deps = "dependencies {\n    testCompile 'a:b:1'\n    compile 'a:b:1'\n}\n";
        for plugin in [
            "apply plugin: 'java'\n",
            "apply(plugin: 'groovy')\n",
            "allprojects { apply plugin: 'war' }\n",
            "plugins {\n    id 'java-library'\n}\n",
            "plugins {\n    id('application')\n}\n",
            "plugins {\n    id 'org.jetbrains.kotlin.jvm' version '1.9.0'\n}\n",
            "plugins {\n    id 'com.android.application'\n}\n",
            "apply plugin: 'com.android.library'\n",
            "plugins {\n    id 'com.android.dynamic-feature'\n}\n",
            "plugins {\n    id 'com.android.test'\n}\n",
            "plugins {\n    id 'io.ratpack.ratpack-java' version '1.9.0'\n}\n",
            "apply plugin: 'io.ratpack.ratpack-groovy'\n",
        ] {
            let src = format!("{plugin}{deps}");
            assert_eq!(
                fixed("build.gradle", Some("implementation"), &src),
                src.replace("testCompile", "testImplementation")
                    .replace("compile '", "implementation '"),
                "{plugin}"
            );
        }
        // none in the file (a parent's allprojects, a convention plugin), or not a Java one:
        // reported, saying why, and left alone
        for plugin in [
            "",
            "apply plugin: 'base'\n",
            "plugins {\n    id 'com.example.java-conventions'\n}\n",
            "dependencies { id 'java' }\n",
        ] {
            let src = format!("{plugin}{deps}");
            let vs = lint_file(Path::new("build.gradle"), &src).unwrap();
            let mut vs: Vec<_> = vs
                .into_iter()
                .filter(|v| v.rule == "GradleRemovedConfiguration")
                .collect();
            crate::lint::opt_in_fixes(&mut vs, &src, Some("implementation"));
            assert_eq!(vs.len(), 2, "{plugin}");
            assert!(
                vs.iter().all(|v| v.fix.is_none()
                    && v.message.ends_with("(not fixed: this file applies no Java plugin, so the new configuration may not exist)")),
                "{plugin}: {vs:?}"
            );
            assert_eq!(fixed("build.gradle", Some("implementation"), &src), src);
        }
    }

    #[test]
    fn compile_becomes_api_only_under_java_library() {
        let deps = "dependencies {\n    compile 'a:b:1'\n}\n";
        for plugin in [
            "apply plugin: 'java'\n",
            "plugins {\n    id 'application'\n}\n",
            "plugins {\n    id 'com.android.application'\n}\n",
        ] {
            let src = format!("{plugin}{deps}");
            let mut vs = lint_file(Path::new("build.gradle"), &src).unwrap();
            crate::lint::opt_in_fixes(&mut vs, &src, Some("api"));
            assert_eq!(
                vs[0].message,
                "The `compile` configuration was removed in Gradle 7.0; use api (`gradle_compile_replacement`) (not fixed: api needs the java-library or com.android.library plugin, which this file doesn't apply)"
            );
            assert_eq!(vs[0].fix, None);
            assert_eq!(fixed("build.gradle", Some("api"), &src), src);
            // implementation needs only java
            assert_eq!(
                fixed("build.gradle", Some("implementation"), &src),
                src.replace("compile", "implementation")
            );
        }
        for plugin in [
            "apply plugin: 'java'\napply plugin: 'java-library'\n",
            "plugins {\n    id 'com.android.library'\n}\n",
        ] {
            let src = format!("{plugin}{deps}");
            assert_eq!(
                fixed("build.gradle", Some("api"), &src),
                src.replace("compile", "api"),
                "{plugin}"
            );
        }
    }

    #[test]
    fn left_shift_is_fixed_only_with_nothing_between_its_parts() {
        // a comment between `<<` and `{`, or a line break anywhere, would be lost or moved
        for src in [
            "task g << /* keep me */ { println 'g' }\n",
            "task h << // note\n{ println 'h' }\n",
            "task x\n<< { println 1 }\n",
            "task y \\\n<< { println 2 }\n",
            "task z <<\n{ println 3 }\n",
        ] {
            assert_eq!(
                rules("build.gradle", src),
                vec![("GradleLeftShiftTask", 1)],
                "{src}"
            );
            assert_eq!(fixed("build.gradle", None, src), src, "{src}");
        }
        // spaces and tabs are fine, and a comment before `<<` stays where it was
        assert_eq!(
            fixed("build.gradle", None, "task a <<\t{ }\ntask e/*x*/<< { }\n"),
            "task a { doLast { } }\ntask e/*x*/{ doLast { } }\n"
        );
    }

    #[test]
    fn deeply_nested_fixes_finish_in_one_run() {
        let depth = 9;
        let names: Vec<String> = (0..depth).map(|i| format!("t{i}")).collect();
        let src = names
            .iter()
            .map(|n| format!("task {n} << {{ "))
            .collect::<String>()
            + "println 1"
            + &" }".repeat(depth)
            + "\n";
        let want = names
            .iter()
            .map(|n| format!("task {n} {{ doLast {{ "))
            .collect::<String>()
            + "println 1"
            + &" } }".repeat(depth)
            + "\n";
        // `fixed` also checks that a second run fixes nothing
        assert_eq!(fixed("build.gradle", None, &src), want);
    }
}
