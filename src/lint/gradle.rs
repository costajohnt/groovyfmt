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
//! dependency notation deprecated in 9.1, "Upgrading from 9.0.0 and earlier"),
//! https://docs.gradle.org/current/userguide/upgrading_major_version_9.html (plugin
//! conventions removed in 9.0, "Removal of conventions").

use super::jenkins::{Call, call, statements};
use super::{Edit, Rule, Violation, at, constant_value, has_interpolation, own_lines};
use crate::text;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::Node;

mod context;
use context::Build;
pub(crate) use context::Cache;
#[cfg(test)]
use context::MAX_WRAPPER_BYTES;

pub(super) const RULES: &[Rule] = &[
    // Gradle build scripts (src/lint/gradle.rs)
    Rule {
        name: "GradleRemovedConfiguration",
        default: true,
        description: "compile, runtime, testCompile or testRuntime dependencies (removed in Gradle 7.0), unless the Gradle wrapper pins an older Gradle",
        why: "`compile`, `runtime`, `testCompile` and `testRuntime` were removed in Gradle 7.0; the build fails on a current Gradle. `--fix` renames `testCompile` to testImplementation, `runtime` to runtimeOnly and `testRuntime` to testRuntimeOnly. `compile` becomes implementation for most builds but api for a dependency a library's own API exposes, so `--fix` replaces it only with `gradle_compile_replacement = \"implementation\"` (or `\"api\"`) in the config's `[lint]` table. The new configurations come from the Java plugins, so a rename is fixed only in a file that applies one (java, java-library, groovy, scala, application, war, java-gradle-plugin or Kotlin JVM, with `apply plugin:` or in `plugins { }`); a file whose plugins come from elsewhere (a parent's allprojects, a convention plugin) is only reported, and the message says so. Where nothing shows a Java plugin may be there (neither the file nor another `.gradle` script in its directory or above it applies one or a script, and the file uses no configuration they make, such as `implementation`), a plugin from elsewhere made the configuration (gradle-hdvl's SystemVerilog plugin makes `compile` and `testCompile`), so nothing is reported. `api` comes only with the java-library plugin, so `compile` becomes api only in a file that applies it. Source: the Gradle upgrade guide for 6.x to 7.0, and the Java Library plugin chapter.",
        bad: "plugins {\n    id 'java'\n}\ndependencies {\n    testCompile 'junit:junit:4.13.2'\n}",
        good: "plugins {\n    id 'java'\n}\ndependencies {\n    testImplementation 'junit:junit:4.13.2'\n}",
        fixable: true,
    },
    Rule {
        name: "GradleJcenter",
        default: true,
        description: "the jcenter() repository (removed in Gradle 9.0)",
        why: "JCenter is read-only and `jcenter()` was removed in Gradle 9.0. `--fix` turns a plain `jcenter()` line in a repositories block into mavenCentral(), or removes it when the block already has mavenCentral(); `jcenter { ... }` is left to you. A dependency published only to JCenter still needs another home. Source: the Gradle upgrade guides for 6.x to 7.0 and 8.x to 9.0.",
        bad: "repositories {\n    jcenter()\n}",
        good: "repositories {\n    mavenCentral()\n}",
        fixable: true,
    },
    Rule {
        name: "GradleLeftShiftTask",
        default: true,
        description: "task name << { } (removed in Gradle 5.0)",
        why: "`task name << { }` was removed in Gradle 5.0; the build fails on a current Gradle. `--fix` writes `task name { doLast { ... } }`, keeping the action as written (`task 'name' << { }` is left to you). Source: the Gradle upgrade guide for 4.x to 5.0.",
        bad: "task hello << {\n    println 'hello'\n}",
        good: "task hello { doLast {\n    println 'hello'\n} }",
        fixable: true,
    },
    Rule {
        name: "GradleDynamicVersion",
        default: false,
        description: "a + or latest.* dependency version (off by default: a policy choice)",
        why: "A `+` or `latest.*` version resolves to whatever is newest at build time, so the same commit can build differently tomorrow.",
        bad: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.+'\n}",
        good: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.0.9'\n}",
        fixable: false,
    },
    Rule {
        name: "GradleRemovedApi",
        default: true,
        description: "a removed core plugin (maven, findbugs, osgi, ...), the uploadArchives task, archive task properties removed in 8.0, configurations.compile and the like, a class Groovy 4 moved out of groovy.util (XmlSlurper, XmlParser, FileNameFinder), or an org.gradle.util class Gradle 9.0 removed (VersionNumber, ...), unless the Gradle wrapper pins an older Gradle",
        why: "These were removed from Gradle: the `maven` plugin and `uploadArchives` in 7.0 (use maven-publish), findbugs, jdepend, osgi and the announce plugins in 6.0, the `compile`, `runtime`, `testCompile` and `testRuntime` configurations in 7.0, an archive task's `archiveName`, `baseName`, `classifier`, `destinationDir`, `appendix` and `extension` in 8.0 (use archiveFileName, archiveBaseName, archiveClassifier, destinationDirectory and so on), the project's `libsDir` and `distsDir` in 7.0 (read base.libsDirectory), and the plugin conventions' project properties in 9.0, set or read (`sourceCompatibility = 11`, `println project.libsDirName`; use `java { }`, `base { }`, `application { }`), with a report's `destination`, a copy spec's `fileMode` and `dirMode`, the IDEA module's `testSourceDirs`, `exec { }` on a project or script and internal `org.gradle.util` classes such as `VersionNumber` and `DistributionLocator` (an import fails). Gradle 9.0 also runs scripts with Groovy 4, which moved `XmlSlurper`, `XmlParser` and `groovy.util.slurpersupport` to groovy.xml and `FileNameFinder` to groovy.ant (--fix adds the import). The build fails on a Gradle without them. Source: the Gradle upgrade guides for 5.x to 6.0, 6.x to 7.0, 7.x to 8.0 and 8.x to 9.0.",
        bad: "apply plugin: 'java'\n\njar {\n    classifier = 'all'\n}",
        good: "apply plugin: 'java'\n\njar {\n    archiveClassifier = 'all'\n}",
        fixable: false,
    },
    Rule {
        name: "GradleSpaceAssignment",
        default: true,
        description: "`description 'x'`, `group 'x'` or `version 'x'` setting the project's property (at the top level or in allprojects, subprojects or project(':x')), or `description 'x'` or `group 'x'` in a task's block: Groovy DSL space assignment, deprecated in Gradle 8.12, unless the Gradle wrapper pins an older Gradle",
        why: "Gradle 8.12 deprecated setting a property as `name value`, a method Gradle generates for each property, and Gradle 10 removes it: write `name = value`. Real methods keep working (`apply plugin: 'java'`, `id 'x' version '1'`, `implementation 'a:b:1'`), so only properties known to have no method of the name are reported, where the script's delegate is known: the project's `description`, `group` and `version`, and a task's `description` and `group`. `--fix` writes `name = value`. Source: the Gradle upgrade guide for 8.11 to 8.12 (\"Groovy space assignment syntax\").",
        bad: "group 'org.example'\nversion '1.0'\n\ntask docs {\n    description 'Builds the docs'\n}",
        good: "group = 'org.example'\nversion = '1.0'\n\ntask docs {\n    description = 'Builds the docs'\n}",
        fixable: true,
    },
    Rule {
        name: "GradleDependencyTuple",
        default: false,
        description: "a dependency in map notation, `implementation group: 'a', name: 'b', version: '1'`, with only group, name and version as plain strings (off by default: a style choice before Gradle 9.1)",
        why: "The string `'a:b:1'` says the same in one place, and Gradle 9.1 deprecated the map (multi-string) notation, to be removed in Gradle 10. Not reported with other keys (`classifier`, `ext`, `configuration`, ...) or a value that isn't a plain string. `--fix` writes the string. Source: the Gradle upgrade guide for 9.0 to 9.1 (\"Deprecation of multi-string dependency notation\").",
        bad: "dependencies {\n    implementation group: 'org.slf4j', name: 'slf4j-api', version: '2.0.9'\n}",
        good: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.0.9'\n}",
        fixable: true,
    },
    Rule {
        name: "GradleDependencyParentheses",
        default: false,
        description: "a dependency in parentheses with no closure after it, `implementation('a:b:1')` (off by default: a style choice)",
        why: "In the Groovy DSL `implementation 'a:b:1'` is the same call, and most build scripts write it that way. Not reported with a closure after it, over several lines, or when an argument would read differently without the parentheses. `--fix` removes them.",
        bad: "dependencies {\n    implementation('org.slf4j:slf4j-api:2.0.9')\n}",
        good: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.0.9'\n}",
        fixable: true,
    },
];

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
    // extend the Groovy plugin (GrailsGradlePlugin extends GroovyPlugin, grails-gradle-plugin 6.2)
    "org.grails.grails-core",
    "org.grails.grails-web",
    "org.grails.grails-plugin",
    // apply java (and groovy), per the Ratpack manual's Gradle chapter
    "io.ratpack.ratpack-java",
    "io.ratpack.ratpack-groovy",
];

/// Configurations the Java plugins make, which a file declaring dependencies in shows it has.
const JAVA_CONFIGURATIONS: &[&str] = &[
    "implementation",
    "api",
    "compileOnly",
    "runtimeOnly",
    "testImplementation",
    "testCompileOnly",
    "testRuntimeOnly",
    "annotationProcessor",
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

pub(super) fn check(
    path: &Path,
    src: &str,
    root: Node,
    cache: Option<&Cache>,
    v: &mut Vec<Violation>,
) {
    if path.extension().is_none_or(|e| e != "gradle") {
        return;
    }
    let build = Build::new(path, cache);
    let mut cx = GradleCtx::gather(src, root, &build, v);
    dependencies(&cx, v);
    execs_and_configuration_refs(&mut cx);
    if cx.methods.contains("jcenter") {
        v.retain(|x| x.rule != "GradleJcenter");
    }
    // a wrapper pinning a Gradle below 5.0 still has `task name << { }`; with no wrapper the
    // report stays, as for GradleRemovedConfiguration
    if v.iter().any(|x| x.rule == "GradleLeftShiftTask")
        && build.wrapper_major().is_some_and(|m| m < 5)
    {
        v.retain(|x| x.rule != "GradleLeftShiftTask");
    }
    convention_sets(&mut cx, v);
    convention_reads(&mut cx);
    let groovy4 = std::mem::take(&mut cx.groovy4);
    if !groovy4.uses.is_empty() && build.wrapper_major().is_some_and(|m| m >= 9) {
        groovy4.report(src, root, v);
    }
    removed_api_reports(cx, v);
}

/// What one pass over a build script gathers for the rules that need the whole script.
struct GradleCtx<'a> {
    src: &'a str,
    build: &'a Build<'a>,
    /// a settings or init script, which isn't a project's
    project_script: bool,
    /// A build can still declare its own configuration called `compile`: then it's valid.
    declared: HashSet<String>,
    /// the bodies of `dependencies { }` blocks
    deps: Vec<Node<'a>>,
    /// GradleRemovedApi: (node, message, the Gradle major that removed it)
    removed: Vec<(Node<'a>, String, u32)>,
    /// `configurations.compile` and the like (`removed_api`)
    conf_refs: Vec<(Node<'a>, &'a str)>,
    plugins: HashSet<String>,
    /// the script's own methods: its `compile(x)` or `jcenter()` is no configuration (#192)
    methods: HashSet<&'a str>,
    /// removed convention properties the script sets, and the extra properties it makes
    conventions: Vec<(Node<'a>, &'a str, &'static str)>,
    ext: HashSet<&'a str>,
    /// reads of removed convention properties (`READS`), and the names the script tests
    /// with `hasProperty('x')`, which it reads as extra properties (#248)
    reads: Vec<(Node<'a>, &'a str, &'static str, u32)>,
    tested: HashSet<&'a str>,
    /// `exec { }` and `javaexec { }` on the project or script
    execs: Vec<(Node<'a>, &'a str)>,
    applies_script: bool,
    groovy4: Groovy4<'a>,
}

impl<'a> GradleCtx<'a> {
    /// One preorder pass: gathers the context and reports the rules that look at one node
    /// (GradleSpaceAssignment, GradleJcenter, GradleLeftShiftTask).
    fn gather(
        src: &'a str,
        root: Node<'a>,
        build: &'a Build<'a>,
        v: &mut Vec<Violation>,
    ) -> GradleCtx<'a> {
        let name = build.path().file_name().and_then(|f| f.to_str());
        let mut cx = GradleCtx {
            src,
            build,
            project_script: name
                .is_some_and(|f| f != "settings.gradle" && !f.ends_with("init.gradle")),
            declared: HashSet::new(),
            deps: Vec::new(),
            removed: Vec::new(),
            conf_refs: Vec::new(),
            plugins: HashSet::new(),
            methods: HashSet::new(),
            conventions: Vec::new(),
            ext: HashSet::new(),
            reads: Vec::new(),
            tested: HashSet::new(),
            execs: Vec::new(),
            applies_script: false,
            groovy4: Groovy4::default(),
        };
        let settings_or_init =
            name.is_some_and(|f| f == "settings.gradle" || f.ends_with("init.gradle"));
        let deprecated_space = std::cell::OnceCell::new();
        crate::preorder(root, |n| {
            cx.groovy4.visit(src, n);
            cx.plugins.extend(applied_plugin(src, n));
            cx.applies_script |= apply_value(src, n, "from").is_some();
            if n.kind() == "method_declaration"
                && let Some(m) = n.child_by_field_name("name")
            {
                cx.methods.insert(text(m, src));
            }
            removed_api(src, n, &mut cx.removed, &mut cx.conf_refs);
            cx.execs.extend(project_exec(src, n));
            convention(src, n, &mut cx.conventions, &mut cx.ext);
            cx.reads.extend(convention_read(src, n));
            cx.tested.extend(has_property(src, n));
            if let Some(names) = property_scope(src, n)
                && !settings_or_init
            {
                for s in statements(n) {
                    space_assignment(src, s, names, &deprecated_space, build, v);
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
                cx.deps.push(b);
            }
            declares(src, n, &mut cx.declared);
            true
        });
        cx
    }
}

/// The statements of the `dependencies { }` blocks: GradleRemovedConfiguration, and the
/// notation rules (GradleDynamicVersion, GradleDependencyParentheses,
/// GradleDependencyTuple), statement by statement.
fn dependencies(cx: &GradleCtx, v: &mut Vec<Violation>) {
    let (src, build) = (cx.src, cx.build);
    // read only for a file that would get a report
    let old_gradle = std::cell::OnceCell::new();
    let jvm = cx.plugins.iter().any(|p| JVM_PLUGINS.contains(&p.as_str()));
    // the Java plugins may be there: the file applies a script, or declares dependencies in
    // a configuration they make (`implementation`), or a nearby script may apply them
    let java_maybe = jvm
        || cx.applies_script
        || cx
            .deps
            .iter()
            .flat_map(|b| statements(*b))
            .any(|s| call(s, src).is_some_and(|c| JAVA_CONFIGURATIONS.contains(&c.name)));
    let jvm_nearby = std::cell::OnceCell::new();
    for &b in &cx.deps {
        for s in statements(b) {
            let Some(c) = call(s, src) else { continue };
            if c.args.is_empty() {
                continue;
            }
            // only the project's own `dependencies { }`, at the top level or in a block that
            // configures a project: `buildscript { dependencies { } }` only has `classpath`
            // (a `compile` there was never valid, and `implementation` isn't either), and
            // NeoGradle's `runs { configureEach { dependencies { runtime x } } }` has only
            // `runtime` (#242)
            if let Some((_, new)) = REMOVED.iter().find(|(old, _)| *old == c.name)
                && !cx.declared.contains(c.name)
                && !cx.methods.contains(c.name)
                && owners(s, src).skip(1).all(|o| PROJECT_SCOPES.contains(&o))
                && !*old_gradle.get_or_init(|| build.wrapper_major().is_some_and(|m| m < 7))
                && !build.declared().contains(c.name)
                && (java_maybe || *jvm_nearby.get_or_init(|| build.jvm_plugin_nearby()))
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
}

/// GradleRemovedApi on `exec { }` and `javaexec { }` on the project or script, and on
/// `configurations.compile` and the other removed configurations, into `cx.removed`.
fn execs_and_configuration_refs(cx: &mut GradleCtx) {
    let src = cx.src;
    // every script: a settings or init script's `exec { }` fails on 9.x as a build script's does
    for (n, name) in std::mem::take(&mut cx.execs) {
        if !cx.methods.contains(name) {
            cx.removed.push((
                n,
                format!("`{name} {{ }}` on the project or build script was removed in Gradle 9.0; inject ExecOperations and call its {name}, or use providers.{name} for a value"),
                9,
            ));
        }
    }
    for (n, name) in std::mem::take(&mut cx.conf_refs) {
        if !cx.declared.contains(name)
            && !guarded(src, n, name)
            && !cx.build.declared().contains(name)
        {
            let new = REMOVED
                .iter()
                .find(|(old, _)| *old == name)
                .map_or("", |r| r.1);
            cx.removed.push((
                n,
                format!("The `{name}` configuration was removed in Gradle 7.0, so `configurations.{name}` fails; use {new}, or a resolvable one such as runtimeClasspath"),
                7,
            ));
        }
    }
}

/// GradleRemovedApi on the removed convention properties the script sets, only under a
/// wrapper pinning 9 or later: many builds still pin 8, where these work.
fn convention_sets(cx: &mut GradleCtx, v: &mut Vec<Violation>) {
    let build = cx.build;
    if cx.project_script
        && !cx.conventions.is_empty()
        && build.wrapper_major().is_some_and(|m| m >= 9)
    {
        for (s, name, new) in std::mem::take(&mut cx.conventions) {
            if !cx.ext.contains(name) && !shadowed(cx.src, s, name) && !build.gradle_property(name)
            {
                v.push(at(
                    s,
                    "GradleRemovedApi",
                    format!("The project property `{name}` was removed in Gradle 9.0 with the plugin conventions; use {new}"),
                ));
            }
        }
    }
}

/// GradleRemovedApi on reads of removed convention properties, into `cx.removed`; not of
/// an extra property of the name this script or one above it makes.
fn convention_reads(cx: &mut GradleCtx) {
    if !cx.project_script {
        return;
    }
    let build = cx.build;
    for (n, name, new, since) in std::mem::take(&mut cx.reads) {
        if !cx.ext.contains(name)
            && !cx.tested.contains(name)
            && !shadowed(cx.src, n, name)
            && !build.gradle_property(name)
            && !build.ext_names().contains(name)
        {
            let with = if since == 9 {
                " with the plugin conventions"
            } else {
                ""
            };
            cx.removed.push((
                n,
                format!("The project property `{name}` was removed in Gradle {since}.0{with}, so reading it fails; read {new}"),
                since,
            ));
        }
    }
}

/// The GradleRemovedApi reports gathered in `cx.removed`, by the wrapper's version.
fn removed_api_reports(cx: GradleCtx, v: &mut Vec<Violation>) {
    if cx.removed.is_empty() {
        return;
    }
    let major = cx.build.wrapper_major();
    for (n, msg, since) in cx.removed {
        // quiet under a wrapper pinning a Gradle that still has it; without a wrapper, a
        // current Gradle (8 or 9, which have none of the pre-9 removals) runs the build. A 9.0
        // removal needs a wrapper pinning 9 or later, as for the conventions: many builds still
        // pin 8.
        if major.is_none_or(|m| m >= since) && (since < 9 || major.is_some()) {
            v.push(at(n, "GradleRemovedApi", msg));
        }
    }
}

/// Classes Groovy 4 moved out of `groovy.util`, and the package they are in now (in Groovy 3
/// too). Gradle 9.0 runs scripts with Groovy 4, so a bare `new XmlSlurper()`, which found
/// `groovy.util.XmlSlurper` through Groovy's default imports, fails with "unable to resolve
/// class XmlSlurper", as do `import groovy.util.XmlSlurper` and the moved
/// `groovy.util.slurpersupport` package (#245). Each checked under 8.14.4 and 9.9.1, and the
/// new names to work under both.
const GROOVY4_MOVED: &[(&str, &str)] = &[
    ("XmlSlurper", "groovy.xml"),
    ("XmlParser", "groovy.xml"),
    ("FileNameFinder", "groovy.ant"),
    ("slurpersupport", "groovy.xml"),
];

/// GradleRemovedApi on the classes of `GROOVY4_MOVED`, under a wrapper pinning Gradle 9 or
/// later, with what `--fix` does: `groovy.util` becomes the new package in an import or a
/// qualified name, and a bare name gets its import, put after the script's imports or before
/// its first statement (one edit for all its uses, which `--fix` makes once). A bare name the script imports (`import a.XmlSlurper`,
/// `as XmlSlurper`, `import groovy.xml.*`) or declares a class of is left alone.
#[derive(Default)]
struct Groovy4<'a> {
    /// bare names (and what to import), and `groovy.util.` qualified names
    uses: Vec<(Node<'a>, &'a str, &'static str, bool)>,
    /// names imported or declared as classes, and the packages imported with `.*`
    names: HashSet<&'a str>,
    stars: HashSet<&'a str>,
    last_import: Option<Node<'a>>,
}

impl<'a> Groovy4<'a> {
    fn visit(&mut self, src: &'a str, n: Node<'a>) {
        let p = n.parent();
        let field = |f: &str| p.is_some_and(|p| p.child_by_field_name(f) == Some(n));
        match n.kind() {
            "import_declaration" => {
                if p.is_some_and(|p| p.kind() == "source_file") {
                    self.last_import = Some(n);
                }
                let Some(name) = n.child_by_field_name("name") else {
                    return;
                };
                if text(n, src).trim_end_matches(';').trim_end().ends_with('*') {
                    self.stars.insert(text(name, src));
                } else if let Some(a) = n.child_by_field_name("alias") {
                    self.names.insert(text(a, src));
                } else {
                    self.names.insert(last(text(name, src)));
                }
            }
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                if let Some(name) = n.child_by_field_name("name") {
                    self.names.insert(text(name, src));
                }
            }
            "qualified_name" | "qualified_type" => {
                let t = text(n, src);
                if let Some(rest) = t.strip_prefix("groovy.util.")
                    && let Some((_, new)) = GROOVY4_MOVED
                        .iter()
                        .find(|m| rest.split('.').next() == Some(m.0))
                {
                    self.uses.push((n, rest, new, true));
                }
            }
            "type_identifier" | "identifier"
                if !p.is_some_and(|p| matches!(p.kind(), "qualified_name" | "qualified_type"))
                    && !["field", "name", "alias", "key"].iter().any(|f| field(f)) =>
            {
                if let Some((name, new)) =
                    GROOVY4_MOVED.iter().take(3).find(|m| m.0 == text(n, src))
                {
                    self.uses.push((n, name, new, false));
                }
            }
            _ => {}
        }
    }

    fn report(self, src: &str, root: Node, v: &mut Vec<Violation>) {
        // the import goes on a line of its own: after the last import, or before the first
        // statement, at its line's start when only whitespace is before it there
        let pos = match self.last_import {
            Some(i) => src[i.end_byte()..]
                .find('\n')
                .map_or(src.len(), |e| i.end_byte() + e + 1),
            None => {
                let mut c = root.walk();
                let first = root
                    .named_children(&mut c)
                    .find(|x| !x.kind().contains("comment"))
                    .map_or(0, |x| x.start_byte());
                let line = src[..first].rfind('\n').map_or(0, |l| l + 1);
                if src[line..first].trim().is_empty() {
                    line
                } else {
                    first
                }
            }
        };
        for (n, name, new, qualified) in self.uses {
            let (message, fix) = if qualified {
                (
                    format!(
                        "`groovy.util.{name}` was removed in Groovy 4, which Gradle 9.0 runs scripts with; use {new}.{name}, which Gradle 8 has too"
                    ),
                    Some(Edit {
                        range: n.start_byte()..n.start_byte() + "groovy.util".len(),
                        with: new.to_string(),
                    }),
                )
            } else if self.names.contains(name) || self.stars.contains(new) {
                continue;
            } else {
                let lf = if pos == src.len() && !src.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                (
                    format!(
                        "`{name}` was groovy.util's, which Groovy 4 (run by Gradle 9.0) removed; import {new}.{name}, which Gradle 8 has too"
                    ),
                    // the same edit for each use, which `--fix` makes once
                    Some(Edit {
                        range: pos..pos,
                        with: format!("{lf}import {new}.{name}\n"),
                    }),
                )
            };
            v.push(Violation {
                fix,
                ..at(n, "GradleRemovedApi", message)
            });
        }
    }
}

/// Project properties that came from plugin conventions, which Gradle 9.0 removed (the java,
/// base, application, war, ear and project-report plugins'), and their replacements. Each
/// was checked to work under Gradle 8.14.4 and to fail under 9.0.0 and 9.9.1, and each
/// replacement to work under both.
const CONVENTIONS: &[(&str, &str)] = &[
    ("sourceCompatibility", "java { sourceCompatibility = ... }"),
    ("targetCompatibility", "java { targetCompatibility = ... }"),
    ("docsDirName", "java { docsDir = ... }"),
    ("testResultsDirName", "java { testResultsDir = ... }"),
    ("testReportDirName", "java { testReportDir = ... }"),
    ("archivesBaseName", "base { archivesName = ... }"),
    ("distsDirName", "base { distsDirectory = ... }"),
    ("libsDirName", "base { libsDirectory = ... }"),
    ("mainClassName", "application { mainClass = ... }"),
    ("applicationName", "application { applicationName = ... }"),
    (
        "applicationDefaultJvmArgs",
        "application { applicationDefaultJvmArgs = ... }",
    ),
    ("executableDir", "application { executableDir = ... }"),
    ("webAppDirName", "the war task's webAppDirectory"),
    ("appDirName", "the ear task's appDirectory"),
    ("libDirName", "the ear task's libDirName"),
    (
        "projectReportDirName",
        "the report tasks' own output settings",
    ),
];

/// Blocks that run against a project, where a name falls through to the project's
/// properties.
const PROJECT_BLOCKS: &[&str] = &[
    "allprojects",
    "subprojects",
    "project",
    "configure",
    "afterEvaluate",
];

/// GradleRemovedApi on a removed convention property (`CONVENTIONS`) set at the project
/// level: `sourceCompatibility = 11` or `project.archivesBaseName = 'x'` at the top level of
/// the script or in a `PROJECT_BLOCKS` block, and `mainClassName` in `application { }`,
/// `archivesBaseName` in `base { }` or an archive task's block, any but the compatibilities
/// in `java { }`, and any in a plugin callback (`pluginManager.withPlugin('java') { }`,
/// `plugins.withId`, `plugins.withType`), which fall through to the project too (#245). Not
/// `java { sourceCompatibility }` or in another task's block, which have properties of these
/// names. Reads are `convention_read`'s.
/// Also gathers, into `ext`, the names the script makes extra properties of (`ext.x = `,
/// `ext { x = }`), which it can then set.
/// Report-only: `java { sourceCompatibility = v }` reads the names in `v` against the
/// extension first, so moving the value there can change what it means.
fn convention<'a>(
    src: &'a str,
    s: Node<'a>,
    out: &mut Vec<(Node<'a>, &'a str, &'static str)>,
    ext: &mut HashSet<&'a str>,
) {
    let Some(left) = s
        .named_child(0)
        .filter(|a| s.kind() == "expression_statement" && a.kind() == "assignment_expression")
        .and_then(|a| a.child_by_field_name("left"))
    else {
        return;
    };
    let field = left.child_by_field_name("field");
    let object = left.child_by_field_name("object").map(|o| text(o, src));
    // `ext.x = `, `project.ext.x = `
    if let (Some(f), Some(o)) = (field, object)
        && (o == "ext" || o.ends_with(".ext"))
    {
        ext.insert(text(f, src));
    }
    let name = match left.kind() {
        "identifier" => text(left, src),
        "field_access" if object == Some("project") => field.map_or("", |f| text(f, src)),
        _ => return,
    };
    // each closure around `s` with the call it is given to and that call's name
    let closures = || {
        std::iter::successors(s.parent(), |p| p.parent())
            .filter(|p| p.kind() == "closure")
            .map(|cl| {
                let m = cl.parent().and_then(|p| {
                    if p.kind() == "argument_list" {
                        p.parent()
                    } else {
                        Some(p)
                    }
                });
                let name = m
                    .and_then(|m| match m.kind() {
                        // `configure([tasks.compileJava]) { }` configures tasks, which have a
                        // sourceCompatibility (#230)
                        // ponytail: an argument naming no project (`configure(libs)`) is skipped
                        // too; follow the variable if that loses real hits
                        "method_invocation" => m.child_by_field_name("function").filter(|f| {
                            text(*f, src) != "configure"
                                || m.child_by_field_name("arguments").is_some_and(|a| {
                                    text(a, src).to_ascii_lowercase().contains("project")
                                })
                        }),
                        // `allprojects { }`
                        "command_chain" => m.child_by_field_name("receiver"),
                        _ => None,
                    })
                    .map_or("", |f| text(f, src));
                (cl, m, name)
            })
    };
    if left.kind() == "identifier"
        && closures()
            .next()
            .is_some_and(|(_, _, o)| o == "ext" || o.ends_with(".ext"))
    {
        ext.insert(name);
    }
    let Some((_, new)) = CONVENTIONS.iter().find(|c| c.0 == name) else {
        return;
    };
    let base = matches!(name, "archivesBaseName" | "distsDirName" | "libsDirName");
    // blocks whose object lacks the property, so it falls through to the project: the java
    // extension has only the compatibilities (#245), a plugin callback's plugin none, an
    // archive task the `archive` names, not these
    let project = closures().all(|(cl, m, o)| {
        PROJECT_BLOCKS.contains(&o)
            || (o == "application" && name == "mainClassName")
            || (o == "base" && base)
            || (o == "java" && !matches!(name, "sourceCompatibility" | "targetCompatibility"))
            || matches!(last(o), "withPlugin" | "withId")
            || matches!(o, "plugins.withType" | "project.plugins.withType")
            || (base && m.and_then(|m| task_body(src, m, ARCHIVE_TYPES, ARCHIVE_TASKS)) == Some(cl))
    });
    if project
        && !std::iter::successors(s.parent(), |p| p.parent()).any(|p| p.kind() == "class_body")
    {
        out.push((s, name, new));
    }
}

/// Project properties that came from plugin conventions, as `CONVENTIONS`, with what to
/// read instead and the Gradle major that removed them. `libsDir` and `distsDir` (read-only)
/// went in 7.0: they work under Gradle 4.10.3 and fail under 7.6, 8.8, 9.0.0 and 9.9.1. The
/// rest went in 9.0: each read (`project.x`, bare at the top level and in an untyped task's
/// `doLast`) was checked to work under 8.8 and to fail under 9.0.0 and 9.9.1, and each
/// replacement to work under 7.6, 8.8 and 9.9.1.
const READS: &[(&str, &str, u32)] = &[
    ("libsDir", "base.libsDirectory", 7),
    ("distsDir", "base.distsDirectory", 7),
    ("archivesBaseName", "base.archivesName", 9),
    ("libsDirName", "base.libsDirectory", 9),
    ("distsDirName", "base.distsDirectory", 9),
    ("sourceCompatibility", "java.sourceCompatibility", 9),
    ("targetCompatibility", "java.targetCompatibility", 9),
    ("docsDirName", "java.docsDir", 9),
    ("docsDir", "java.docsDir", 9),
    ("testResultsDirName", "java.testResultsDir", 9),
    ("testResultsDir", "java.testResultsDir", 9),
    ("testReportDirName", "java.testReportDir", 9),
    ("testReportDir", "java.testReportDir", 9),
    ("mainClassName", "application.mainClass", 9),
    ("applicationName", "application.applicationName", 9),
    (
        "applicationDefaultJvmArgs",
        "application.applicationDefaultJvmArgs",
        9,
    ),
    ("executableDir", "application.executableDir", 9),
    (
        "applicationDistribution",
        "application.applicationDistribution",
        9,
    ),
    ("webAppDirName", "the war task's webAppDirectory", 9),
    ("webAppDir", "the war task's webAppDirectory", 9),
    ("appDirName", "the ear task's appDirectory", 9),
    ("libDirName", "the ear task's libDirName", 9),
    (
        "deploymentDescriptor",
        "the ear task's deploymentDescriptor",
        9,
    ),
    (
        "projectReportDirName",
        "the report tasks' own output settings",
        9,
    ),
    (
        "projectReportDir",
        "the report tasks' own output settings",
        9,
    ),
];

/// GradleRemovedApi on a read of a removed convention property (`READS`): `project.libsDir`
/// anywhere outside a class (also in `"${project.libsDir}"` and `"$project.libsDir"`), or a bare `libsDir` where it
/// falls through to the project: at the top level, in a `PROJECT_BLOCKS` block other than
/// `configure`, and in an untyped task's block or its `doLast { }` / `doFirst { }`, whose
/// `DefaultTask` has no such property. Not in a typed task's block (a JavaCompile has a
/// sourceCompatibility, an Ear a libDirName), an extension's (`java { }`), a script method
/// or a closure with a parameter of that name. `check` drops a name the build makes an
/// extra property of or sets in `gradle.properties`, or a local variable.
fn convention_read<'a>(
    src: &'a str,
    n: Node<'a>,
) -> Option<(Node<'a>, &'a str, &'static str, u32)> {
    let is = |p: Node, field: &str| p.child_by_field_name(field) == Some(n);
    let p = n.parent()?;
    let name = match n.kind() {
        // `project.libsDir`, not `project.libsDir = x` (a set, `convention`) or a call
        "field_access"
            if n.child_by_field_name("object")
                .is_some_and(|o| o.kind() == "identifier" && text(o, src) == "project")
                && !is(p, "left")
                && !is(p, "function")
                && !shadowed(src, n, "project") =>
        {
            text(n.child_by_field_name("field")?, src)
        }
        // `"$libsDir"` or `"$project.libsDir"`: one identifier holding the whole path
        "identifier" if p.kind() == "gstring_dollar_interpolation" => {
            let mut path = text(n, src).trim_start_matches('$').split('.');
            match (path.next()?, path.next()) {
                ("project", Some(f)) if !shadowed(src, n, "project") => f,
                (x, _) if project_scope(src, n) => x,
                _ => return None,
            }
        }
        "identifier"
            if !["field", "function", "left", "key", "receiver", "name"]
                .iter()
                .any(|f| is(p, f))
                && !(p.kind() == "expression_statement"
                    && call(p, src).is_some_and(|c| !c.args.is_empty()))
                && !matches!(p.kind(), "closure_parameter" | "formal_parameter")
                && project_scope(src, n) =>
        {
            text(n, src)
        }
        _ => return None,
    };
    let (name, new, since) = READS.iter().find(|r| r.0 == name)?;
    let in_code = std::iter::successors(n.parent(), |p| p.parent())
        .any(|p| matches!(p.kind(), "class_body" | "method_declaration"));
    (!in_code).then_some((n, *name, *new, *since))
}

/// The name in `hasProperty('x')` or `project.hasProperty('x')`: a script that tests for
/// it treats it as an extra property its subprojects may set (`ext.x = `), which Gradle 9
/// still resolves.
fn has_property<'a>(src: &'a str, n: Node<'a>) -> Option<&'a str> {
    let f = n
        .child_by_field_name("function")
        .filter(|_| n.kind() == "method_invocation")?;
    if !matches!(text(f, src), "hasProperty" | "project.hasProperty") {
        return None;
    }
    let arg = n.child_by_field_name("arguments")?.named_child(0)?;
    let t = text(arg, src);
    (arg.kind() == "string_literal" && !t.contains('$')).then(|| t.trim_matches(['\'', '"']))
}

/// Whether a bare name at `n` falls through to the project (`convention_read`): every
/// closure around it is a `PROJECT_BLOCKS` block other than `configure`, a `withPlugin` or
/// `withId` plugin callback, an untyped task's
/// block, or a `doLast { }` / `doFirst { }` right in one, and none has parameters.
fn project_scope(src: &str, n: Node) -> bool {
    let closures: Vec<Node> = std::iter::successors(n.parent(), |p| p.parent())
        .filter(|p| p.kind() == "closure")
        .collect();
    let names: Vec<&str> = owners(n, src).collect();
    closures
        .iter()
        .zip(&names)
        .enumerate()
        .all(|(i, (cl, name))| {
            let mut c = cl.walk();
            !cl.named_children(&mut c)
                .any(|x| x.kind() == "closure_parameters")
                && ((PROJECT_BLOCKS.contains(name) && *name != "configure")
                    || matches!(*name, "withPlugin" | "withId")
                    || untyped_task(src, *cl)
                    || (matches!(*name, "doLast" | "doFirst")
                        && closures.get(i + 1).is_some_and(|o| untyped_task(src, *o))))
        })
}

/// Whether the closure `cl` configures a task with no type (a `DefaultTask`): `task x { }`,
/// `task x(dependsOn: y) { }`, `task('x') { }` or `tasks.register('x') { }`, not
/// `task x(type: Jar) { }` or `tasks.register('x', Jar) { }`.
fn untyped_task(src: &str, cl: Node) -> bool {
    let Some(p) = cl.parent() else { return false };
    let stmt = p
        .parent()
        .filter(|s| s.kind() == "expression_statement")
        .unwrap_or(p);
    // `task x { }` is two statements, `task` and `x { }`, on one line
    let after_task = stmt.prev_named_sibling().is_some_and(|t| {
        text(t, src) == "task" && t.end_position().row == stmt.start_position().row
    });
    match p.kind() {
        "command_chain" => {
            let mut c = p.walk();
            after_task
                && p.child_by_field_name("receiver")
                    .is_some_and(|r| r.kind() == "identifier")
                && p.children_by_field_name("argument", &mut c).eq([cl])
        }
        "method_invocation" if p.child_by_field_name("closure") == Some(cl) => {
            let Some(f) = p.child_by_field_name("function") else {
                return false;
            };
            let mut positional = 0;
            if let Some(a) = p.child_by_field_name("arguments") {
                let mut c = a.walk();
                for x in a.named_children(&mut c) {
                    if x.kind() != "named_argument" {
                        positional += 1;
                    } else if x
                        .child_by_field_name("key")
                        .is_some_and(|k| text(k, src) == "type")
                    {
                        return false;
                    }
                }
            }
            let f = text(f, src);
            if matches!(
                f,
                "task" | "tasks.register" | "tasks.create" | "project.tasks.register"
            ) {
                positional == 1
            } else {
                after_task && positional == 0
            }
        }
        _ => false,
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

/// Whether `n` applies a plugin from `JVM_PLUGINS` or a script, which may apply one.
fn may_apply_java(src: &str, n: Node) -> bool {
    applied_plugin(src, n).is_some_and(|p| JVM_PLUGINS.contains(&p.as_str()))
        || apply_value(src, n, "from").is_some()
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
/// JavaExec's `main`, removed in Gradle 9.0: checked to work under 8.8 and to fail under
/// 9.0.0 and 9.9.1 ("Could not set unknown property 'main'").
const JAVA_EXEC_PROPS: &[(&str, &str)] = &[("main", "mainClass")];
/// Task types that extend JavaExec.
const JAVA_EXEC_TYPES: &[&str] = &["JavaExec", "BootRun"];
/// JavaExec tasks that Spring Boot's plugin creates under these names.
const JAVA_EXEC_TASKS: &[&str] = &["bootRun"];
/// A copy spec's `fileMode` and `dirMode`, removed in Gradle 9.0: checked to work under
/// 8.14.4 and to fail under 9.9.1, and the replacements (Gradle 8.3 and later) to work under
/// both (#245).
const MODE_PROPS: &[(&str, &str)] = &[
    ("fileMode", "filePermissions { unix(...) }"),
    ("dirMode", "dirPermissions { unix(...) }"),
];
/// Task types that are copy specs: Copy, Sync and the archive tasks.
const COPY_TYPES: &[&str] = &[
    "Copy",
    "Sync",
    "Jar",
    "Zip",
    "Tar",
    "War",
    "Ear",
    "ShadowJar",
    "BootJar",
    "BootWar",
];
/// Copy spec tasks that core plugins (and Spring Boot's and Shadow's) create under these names.
const COPY_TASKS: &[&str] = &[
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
    "processResources",
    "processTestResources",
    "installDist",
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

/// Blocks a project's `dependencies { }` can sit in and still be the project's, with the
/// plugin callbacks: `plugins.withType(JavaPlugin) { }`, `pluginManager.withPlugin('java') { }`,
/// `plugins.withId('java') { }` (datahub-gma).
const PROJECT_SCOPES: &[&str] = &[
    "allprojects",
    "subprojects",
    "project",
    "configure",
    "afterEvaluate",
    "withType",
    "withPlugin",
    "withId",
];

/// The last part of a dotted name: `ShadowJar` of `com.github.jengelman.gradle.plugins.shadow.tasks.ShadowJar`.
fn last(t: &str) -> &str {
    t.rsplit('.').next().unwrap_or(t)
}

/// `org.gradle.util` classes Gradle 9.0 removed: an import of one fails the script's
/// compilation ("unable to resolve class"). Each import checked to work under 8.14.4 and
/// fail under 9.9.1; the package's `GradleVersion`, `Path` and `Configurable` remain (#248).
const REMOVED_UTIL: &[&str] = &[
    "ClosureBackedAction",
    "CollectionUtils",
    "ConfigureUtil",
    "DistributionLocator",
    "GFileUtils",
    "GUtil",
    "NameMatcher",
    "NameValidator",
    "RelativePathUtil",
    "SingleMessageLogger",
    "TextUtil",
    "VersionNumber",
    "WrapUtil",
];

fn removed_api<'a>(
    src: &'a str,
    n: Node<'a>,
    out: &mut Vec<(Node<'a>, String, u32)>,
    conf_refs: &mut Vec<(Node<'a>, &'a str)>,
) {
    // import org.gradle.util.VersionNumber, import static org.gradle.util.GUtil.x
    // ponytail: imports only; a fully qualified use in code is rare, add it if seen
    if n.kind() == "import_declaration"
        && let Some(name) = n.child_by_field_name("name")
        && let Some(class) = text(name, src)
            .strip_prefix("org.gradle.util.")
            .and_then(|r| r.split('.').next())
        && REMOVED_UTIL.contains(&class)
    {
        out.push((
            n,
            format!("`org.gradle.util.{class}` was removed in Gradle 9.0, so importing it fails; use a library or Gradle's public API instead (GradleVersion for Gradle versions)"),
            9,
        ));
    }
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
    let bodies = [
        (
            task_body(src, n, ARCHIVE_TYPES, ARCHIVE_TASKS),
            ARCHIVE_PROPS,
            "archive tasks",
            8,
        ),
        (
            task_body(src, n, JAVA_EXEC_TYPES, JAVA_EXEC_TASKS),
            JAVA_EXEC_PROPS,
            "JavaExec tasks",
            9,
        ),
        (
            task_body(src, n, COPY_TYPES, COPY_TASKS).or_else(|| {
                // `into { dir }` alone is a lazy path, not a child spec
                call(n, src)
                    .filter(|c| match c.name {
                        "copy" | "sync" => c.args.is_empty(),
                        "from" | "into" => !c.args.is_empty() && in_copy_spec(src, n),
                        _ => false,
                    })
                    .and_then(|c| c.body)
            }),
            MODE_PROPS,
            "copy specs",
            9,
        ),
    ];
    for (body, props, tasks, since) in bodies {
        let Some(body) = body else { continue };
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
            if let Some((old, new)) = name.and_then(|x| props.iter().find(|p| p.0 == x)) {
                out.push((
                    s,
                    format!(
                        "The `{old}` property of {tasks} was removed in Gradle {since}.0; use {new}"
                    ),
                    since,
                ));
            }
        }
    }
    if let Some((node, msg)) = report_destination(src, n).or_else(|| idea_test_dirs(src, n)) {
        out.push((node, msg, 9));
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

/// Whether `n`, a child spec's `from(x) { }` or `into(x) { }`, is in a Gradle copy spec: the
/// first block around it that is no other child spec is a Copy, Sync or archive task's
/// (`COPY_TYPES`, `COPY_TASKS`), `copy { }`, `sync { }` or a distribution's `contents { }`.
/// A plugin's own spec (nebula's `ospackage { from(x) { fileMode 0755 } }`, which builds on
/// Gradle 9.8.2) is not.
fn in_copy_spec(src: &str, n: Node) -> bool {
    let closures =
        std::iter::successors(n.parent(), |p| p.parent()).filter(|p| p.kind() == "closure");
    for (cl, name) in closures.zip(owners(n, src)) {
        if matches!(name, "from" | "into") {
            continue;
        }
        let m = cl.parent().and_then(|p| {
            if p.kind() == "argument_list" {
                p.parent()
            } else {
                Some(p)
            }
        });
        return matches!(name, "copy" | "sync" | "contents")
            || m.and_then(|m| task_body(src, m, COPY_TYPES, COPY_TASKS)) == Some(cl);
    }
    false
}

/// The reports of the core tasks' `reports { }` (JaCoCo, Test, Checkstyle, PMD, CodeNarc).
const REPORTS: &[&str] = &["html", "xml", "csv", "junitXml", "sarif", "text"];

/// A report's `destination` set, removed in Gradle 9.0 (#245): `html.destination = f`,
/// `xml.destination f` or `xml.destination(f)` in a `reports { }` block or on `reports.`, and
/// `destination = f` or `destination f` in `reports { html { } }`. Each checked to work under
/// 8.14.4 and fail under 9.9.1, and `outputLocation = f` to work under both. A read is left
/// alone: `destination` is write-only on 8.x too, so a read fails there as well.
fn report_destination<'a>(src: &str, n: Node<'a>) -> Option<(Node<'a>, String)> {
    let p = n.parent()?;
    let mut owners = owners(n, src);
    let set = match n.kind() {
        // `html.destination = f`, `xml.destination(f)`, and `xml.destination f`, which the
        // grammar reads as two statements on one line
        "field_access" => {
            let o = text(n.child_by_field_name("object")?, src);
            let mut path = o.rsplit('.');
            text(n.child_by_field_name("field")?, src) == "destination"
                && REPORTS.contains(&path.next()?)
                && (path.next() == Some("reports")
                    || (!o.contains('.') && owners.next() == Some("reports")))
                && (p.child_by_field_name("left") == Some(n)
                    || p.child_by_field_name("function") == Some(n)
                    || p.kind() == "expression_statement")
        }
        // `reports { html { destination = f } }`
        "expression_statement" => {
            let name = match call(n, src) {
                Some(c) if !c.args.is_empty() && c.body.is_none() => c.name,
                _ => n
                    .named_child(0)
                    .filter(|a| a.kind() == "assignment_expression")
                    .and_then(|a| a.child_by_field_name("left"))
                    .filter(|l| l.kind() == "identifier")
                    .map(|l| text(l, src))?,
            };
            name == "destination"
                && owners.next().is_some_and(|o| REPORTS.contains(&o))
                && owners.next() == Some("reports")
        }
        _ => false,
    };
    set.then(|| {
        (
            n,
            "A report's `destination` was removed in Gradle 9.0; set its outputLocation".into(),
        )
    })
}

/// The IDEA plugin's `testSourceDirs` and `testResourceDirs`, removed in Gradle 9.0 (#245):
/// in `idea { module { } }` or as `idea.module.testSourceDirs`, set or read. Checked to work
/// under 8.14.4 and fail under 9.9.1, and `testSources.from(...)` and
/// `testResources.from(...)` to work under both.
fn idea_test_dirs<'a>(src: &str, n: Node<'a>) -> Option<(Node<'a>, String)> {
    let p = n.parent()?;
    let name = match n.kind() {
        "field_access"
            if n.child_by_field_name("object")
                .is_some_and(|o| text(o, src) == "idea.module") =>
        {
            text(n.child_by_field_name("field")?, src)
        }
        "identifier"
            if !["field", "function", "key", "name"]
                .iter()
                .any(|f| p.child_by_field_name(f) == Some(n))
                && owners(n, src).take(2).eq(["module", "idea"]) =>
        {
            text(n, src)
        }
        _ => return None,
    };
    let new = match name {
        "testSourceDirs" => "testSources.from(...)",
        "testResourceDirs" => "testResources.from(...)",
        _ => return None,
    };
    Some((
        n,
        format!("The IDEA module's `{name}` was removed in Gradle 9.0; use {new}"),
    ))
}

/// `exec { }` or `javaexec { }` (also `exec({ })`, `project.exec { }`,
/// `rootProject.javaexec { }`), which Gradle 9.0 removed from the project and the build
/// script: in a task action, a script method or at the top level it fails with "Could not
/// find method exec()" (checked under 8.8, 9.0.0 and 9.9.1). Only with a closure: a bare
/// `exec()` is an Exec task's own action. Not inside a class, whose own or inherited `exec`
/// it may be, nor on another receiver (`execOperations.exec { }`, `providers.exec { }`),
/// nor in a `with { }`, `tap { }` or `identity { }`, which hands the closure another
/// delegate (gocd's `injected.execOps.with { exec { } }`).
fn project_exec<'a>(src: &'a str, n: Node<'a>) -> Option<(Node<'a>, &'a str)> {
    let (name, closure) = match n.kind() {
        "command_chain" => {
            let mut c = n.walk();
            let args: Vec<_> = n.children_by_field_name("argument", &mut c).collect();
            (
                n.child_by_field_name("receiver")
                    .filter(|r| r.kind() == "identifier")?,
                matches!(args[..], [a] if a.kind() == "closure"),
            )
        }
        "method_invocation" => {
            let f = n.child_by_field_name("function")?;
            let name = match f.kind() {
                "identifier" => f,
                "field_access"
                    if f.child_by_field_name("object")
                        .is_some_and(|o| matches!(text(o, src), "project" | "rootProject")) =>
                {
                    f.child_by_field_name("field")?
                }
                _ => return None,
            };
            let only_closure_arg = n.child_by_field_name("arguments").is_some_and(|a| {
                a.named_child_count() == 1
                    && a.named_child(0).is_some_and(|x| x.kind() == "closure")
            });
            (
                name,
                n.child_by_field_name("closure").is_some() || only_closure_arg,
            )
        }
        _ => return None,
    };
    let name = text(name, src);
    (closure
        && matches!(name, "exec" | "javaexec")
        && !std::iter::successors(n.parent(), |p| p.parent()).any(|p| p.kind() == "class_body")
        && !owners(n, src).any(|o| matches!(o, "with" | "tap" | "identity")))
    .then_some((n, name))
}

/// Whether `n`, a `configurations.<name>`, runs only when an `if` or `?:` above it found
/// the configuration: `if (configurations.hasProperty('compile')) { configurations.compile }`
/// (or `findByName`), which a Gradle without it never evaluates (#222).
fn guarded(src: &str, n: Node, name: &str) -> bool {
    let checks = ["hasProperty", "findByName"]
        .map(|m| [format!("{m}('{name}')"), format!("{m}(\"{name}\")")]);
    let mut inner = n;
    std::iter::successors(n.parent(), |p| p.parent()).any(|p| {
        let child = std::mem::replace(&mut inner, p);
        matches!(p.kind(), "if_statement" | "ternary_expression")
            && p.child_by_field_name("consequence")
                .is_some_and(|c| c.id() == child.id())
            && p.child_by_field_name("condition").is_some_and(|c| {
                let t = text(c, src);
                checks.iter().flatten().any(|k| t.contains(k.as_str()))
            })
    })
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
    apply_value(src, n, key).and_then(|v| constant_value(v, src))
}

/// The string literal the named argument `n` passes to `apply` as `key`, interpolated or not.
fn apply_value<'a>(src: &str, n: Node<'a>, key: &str) -> Option<Node<'a>> {
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
    applies.then_some(val)
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

/// The configuration block of a task of one of `types`: one of the `tasks` names alone
/// (`jar { }`, where it means the task) or on `tasks` (`tasks.jar { }`), `task x(type: Jar) { }`, `tasks.register('x', Zip)
/// { }`, `tasks.withType(Jar) { }` or `tasks.withType(Jar).configureEach { }`.
fn task_body<'a>(src: &'a str, n: Node<'a>, types: &[&str], tasks: &[&str]) -> Option<Node<'a>> {
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
                    && types.contains(&last(text(x, src)))
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
                || ((f.kind() == "identifier"
                    // `tasks.jar { }` (#245)
                    || f.child_by_field_name("object")
                        .is_some_and(|o| matches!(text(o, src), "tasks" | "project.tasks")))
                    && tasks.contains(&last(text(f, src)))
                    && n.child_by_field_name("arguments")
                        .is_none_or(|a| a.named_child_count() == 0)
                    && in_scope());
            ok.then_some(body)
        }
        "command_chain" => {
            let c = call(n, src)?;
            (tasks.contains(&c.name) && c.args.is_empty() && in_scope())
                .then_some(c.body)
                .flatten()
        }
        _ => None,
    }
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
    build: &Build,
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
        || !*deprecated.get_or_init(|| build.wrapper_version().is_none_or(|v| v >= (8, 12)))
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

    /// #242: only the project's own `dependencies { }`: NeoGradle's run dependencies have
    /// only `runtime`, so `--fix` must not make it `runtimeOnly`.
    #[test]
    fn removed_configurations_only_in_project_dependencies() {
        let java = "apply plugin: 'java'\n";
        let neo = format!(
            "{java}runs {{\n    configureEach {{\n        dependencies {{\n            runtime project.configurations.libraries\n        }}\n    }}\n}}\n"
        );
        assert_eq!(rules("build.gradle", &neo), vec![]);
        assert_eq!(fixed("build.gradle", None, &neo), neo);
        let suite = format!(
            "{java}testing {{ suites {{ test {{ dependencies {{ testCompile 'a:b:1' }} }} }} }}\n"
        );
        assert_eq!(rules("build.gradle", &suite), vec![]);
        // a block that configures a project: still reported, and fixed
        for scope in [
            "allprojects",
            "subprojects",
            "project(':a')",
            "configure(subprojects)",
            "afterEvaluate",
            "plugins.withType(JavaPlugin)",
            "pluginManager.withPlugin('java')",
            "plugins.withId('java')",
        ] {
            let src = format!(
                "{java}{scope} {{\n    dependencies {{\n        runtime 'a:b:1'\n    }}\n}}\n"
            );
            assert_eq!(
                rules("build.gradle", &src),
                vec![("GradleRemovedConfiguration", 4)],
                "{scope}"
            );
            assert!(
                fixed("build.gradle", None, &src).contains("runtimeOnly 'a:b:1'"),
                "{scope}"
            );
        }
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
        let build = |s: &str| {
            std::fs::write(
                root.join("build.gradle"),
                format!("apply plugin: 'java'\n{s}"),
            )
            .unwrap()
        };
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

    /// #241: a root `build.gradle` that makes `compile` for every project (corretto-11).
    #[test]
    fn removed_configurations_declared_for_every_project() {
        let root =
            std::env::temp_dir().join(format!("groovyfmt-allprojects-{}", std::process::id()));
        let dir = root.join("installers/linux/spec");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            root.join("settings.gradle"),
            "include ':installers:linux:spec'\n",
        )
        .unwrap();
        let java = "project(':installers:linux:spec') {\n    apply plugin: 'java'\n}\n";
        let uses =
            "dependencies {\n    compile files('a.txt')\n}\ndef f = configurations.compile.files\n";
        let p = dir.join("build.gradle");
        // through `Projects`, the scripts above are read once while unchanged
        let projects = crate::lint::Projects::new(None);
        let cached = || -> Vec<_> {
            crate::lint::lint_with(&p, uses, &projects.spec_base(&p))
                .unwrap()
                .into_iter()
                .filter(|v| v.rule.starts_with("Gradle"))
                .map(|v| (v.rule, v.line))
                .collect()
        };
        let run = |parent: &str| {
            std::fs::write(root.join("build.gradle"), format!("{parent}{java}")).unwrap();
            std::fs::write(&p, uses).unwrap();
            projects.clear();
            let got = rules(p.to_str().unwrap(), uses);
            assert_eq!(cached(), got, "{parent}");
            got
        };
        for parent in [
            "allprojects {\n    apply plugin: 'base'\n    configurations {\n        compile {\n            transitive = false\n        }\n    }\n}\n",
            "subprojects {\n    configurations { compile }\n}\n",
            "subprojects {\n    configurations.create('compile')\n}\n",
            "allprojects {\n    configurations { create('compile') }\n}\n",
        ] {
            assert_eq!(run(parent), vec![], "{parent}");
        }
        // the root project's own configuration, another one, or none: reported as before
        let reported = vec![("GradleRemovedConfiguration", 2), ("GradleRemovedApi", 4)];
        for parent in [
            "configurations { compile }\n",
            "subprojects {\n    configurations { provided }\n}\n",
            "",
        ] {
            assert_eq!(run(parent), reported, "{parent}");
        }
        // a script changed on disk is read again, without `clear`
        std::fs::write(
            root.join("build.gradle"),
            "subprojects { configurations { compile } }\n",
        )
        .unwrap();
        assert_eq!(cached(), vec![]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// #230: gradle-hdvl's SystemVerilog plugin makes `compile` and `testCompile`.
    #[test]
    fn removed_configurations_need_a_java_plugin_nearby() {
        let root = std::env::temp_dir().join(format!("groovyfmt-hdvl-{}", std::process::id()));
        std::fs::create_dir_all(root.join("proj1")).unwrap();
        std::fs::create_dir_all(root.join("tb")).unwrap();
        std::fs::write(root.join("settings.gradle"), "include 'proj1', 'tb'\n").unwrap();
        let parent = |s: &str| std::fs::write(root.join("build.gradle"), s).unwrap();
        let run = |dir: &str, src: &str| {
            let p = root.join(dir).join("build.gradle");
            std::fs::write(&p, src).unwrap();
            rules(p.to_str().unwrap(), src)
        };
        let proj1 = "dependencies {\n    compile project(':proj0')\n}\n";
        let tb = "plugins {\n    id 'com.verificationgentleman.gradle.hdvl.systemverilog'\n}\ndependencies {\n    testCompile 'org.svunit:svunit:v3.34.2'\n}\n";
        parent(
            "plugins {\n    id 'com.verificationgentleman.gradle.hdvl.systemverilog' apply false\n}\nsubprojects {\n    apply plugin: 'com.verificationgentleman.gradle.hdvl.systemverilog'\n    dependencies {\n        testCompile 'a:b:1'\n    }\n}\n",
        );
        assert_eq!(run("proj1", proj1), vec![]);
        assert_eq!(run("tb", tb), vec![]);
        assert_eq!(
            run(
                ".",
                &std::fs::read_to_string(root.join("build.gradle")).unwrap()
            ),
            vec![]
        );
        // the file applies a script, or uses a configuration the Java plugins make: they may
        // be there
        let applies = "apply from: \"${rootDir}/gradle/java.gradle\"\ndependencies {\n    compile 'a:b:1'\n}\n";
        assert_eq!(
            run("proj1", applies),
            vec![("GradleRemovedConfiguration", 3)]
        );
        let uses = "dependencies {\n    compile project(':sdk')\n    implementation 'a:b:1'\n}\n";
        assert_eq!(run("proj1", uses), vec![("GradleRemovedConfiguration", 2)]);
        // a Grails plugin applies the Groovy plugin, so the rename is fixed
        let grails =
            "apply plugin: 'org.grails.grails-web'\ndependencies {\n    testCompile 'a:b:1'\n}\n";
        let fixed = lint_file(&root.join("tb/build.gradle"), grails).unwrap();
        assert!(
            fixed
                .iter()
                .any(|v| v.rule == "GradleRemovedConfiguration" && v.fix.is_some())
        );
        // a parent that applies java makes them, so they are reported, unfixed
        parent("subprojects {\n    apply plugin: 'java'\n}\n");
        assert_eq!(run("proj1", proj1), vec![("GradleRemovedConfiguration", 2)]);
        assert_eq!(run("tb", tb), vec![("GradleRemovedConfiguration", 5)]);
        // or applies a script, which may
        parent("subprojects {\n    apply from: \"$rootDir/gradle/java.gradle\"\n}\n");
        assert_eq!(run("proj1", proj1), vec![("GradleRemovedConfiguration", 2)]);
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
    fn left_shift_tasks_under_a_wrapper() {
        // a Gradle below 5.0 still has `task name << { }` (#256)
        let root = std::env::temp_dir().join(format!("groovyfmt-lshift-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let run = |v: &str| {
            std::fs::write(
                root.join("gradle/wrapper/gradle-wrapper.properties"),
                format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n"),
            )
            .unwrap();
            rules(
                root.join("build.gradle").to_str().unwrap(),
                "task hello << { }\n",
            )
            .len()
        };
        assert_eq!((run("2.2"), run("4.6"), run("4.10.3")), (0, 0, 0));
        assert_eq!((run("5.0"), run("8.5")), (1, 1));
        std::fs::remove_dir_all(&root).unwrap();
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
    fn removed_gradle_util() {
        // #248: org.gradle.util classes removed in 9.0, only under a wrapper pinning 9
        let root = std::env::temp_dir().join(format!("groovyfmt-util-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let props = root.join("gradle/wrapper/gradle-wrapper.properties");
        let wrapper = |v: &str| {
            std::fs::write(&props, format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n")).unwrap()
        };
        let lines = |src: &str| {
            rules(root.join("build.gradle").to_str().unwrap(), src)
                .into_iter()
                .filter(|r| r.0 == "GradleRemovedApi")
                .map(|r| r.1)
                .collect::<Vec<_>>()
        };
        let src = "import org.gradle.util.DistributionLocator\nimport org.gradle.util.VersionNumber\nimport static org.gradle.util.GUtil.toCamelCase\nimport org.gradle.util.TextUtil as T\nimport org.gradle.util.GradleVersion\nimport org.gradle.util.Path\nimport org.gradle.util.*\nimport org.gradle.util.internal.VersionNumber\nimport com.x.org.gradle.util.GUtil\n";
        wrapper("9.9.1");
        assert_eq!(lines(src), vec![1, 2, 3, 4]);
        wrapper("8.14.4");
        assert_eq!(lines(src), Vec::<usize>::new());
        std::fs::remove_dir_all(&root).unwrap();
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

    /// Gradle 9.0's convention removals (#218), each checked under Gradle 8.14.4 and 9.0.0.
    #[test]
    fn removed_conventions() {
        let root =
            std::env::temp_dir().join(format!("groovyfmt-convention-{}", std::process::id()));
        let sub = root.join("sub");
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        let props = root.join("gradle/wrapper/gradle-wrapper.properties");
        let wrapper = |v: &str| {
            std::fs::write(&props, format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n")).unwrap()
        };
        let lines = |dir: &Path, name: &str, src: &str| {
            rules(dir.join(name).to_str().unwrap(), src)
                .iter()
                .filter(|r| r.0 == "GradleRemovedApi")
                .map(|r| r.1)
                .collect::<Vec<_>>()
        };
        let src = "apply plugin: 'application'\nsourceCompatibility = 1.8\nproject.targetCompatibility = '11'\nmainClassName = 'a.B'\narchivesBaseName = 'x'\napplicationDefaultJvmArgs += ['-X']\nif (ci) { libsDirName = 'l' }\nallprojects { docsDirName = 'd' }\nsubprojects {\n    afterEvaluate { webAppDirName = 'w' }\n}\nproject(':a') { appDirName = 'a' }\napplication { mainClassName = 'a.B' }\nbase { archivesBaseName = 'y' }\n";
        wrapper("9.0.0");
        assert_eq!(
            lines(&root, "build.gradle", src),
            vec![2, 3, 4, 5, 6, 7, 8, 10, 12, 13, 14]
        );
        // under Gradle 8, or with no wrapper, quiet: many builds still pin 8
        wrapper("8.14.4");
        assert_eq!(lines(&root, "build.gradle", src), Vec::<usize>::new());
        assert_eq!(rules("build.gradle", src), vec![]);
        wrapper("9.9.1");
        // the new forms, a task's own properties, a read, an extra property the script
        // makes, a local variable, a class's code, a settings script
        let quiet = "java {\n    sourceCompatibility = 11\n    targetCompatibility = 11\n}\napplication {\n    mainClass = 'a.B'\n    applicationName = 'x'\n    executableDir = 'b'\n}\nbase { archivesName = 'x' }\ncompileJava { sourceCompatibility = 11 }\ntasks.withType(JavaCompile) { targetCompatibility = 11 }\ntasks.named('x').configure { sourceCompatibility = 11 }\nshadowJar { mainClassName = 'a.B' }\njar { manifest { attributes 'Main-Class': mainClassName } }\next.executableDir = 'b'\nexecutableDir = 'c'\next {\n    webAppDirName = 'w'\n}\nwebAppDirName = 'v'\ndef appDirName = 'a'\nappDirName = 'z'\nclass C { def f() { libDirName = 1 } }\nfoo.mainClassName = 'x'\nconfigure([tasks.compileJava]) {\n    sourceCompatibility = 25\n}\nconfigure(compileTestJava) { targetCompatibility = 11 }\n";
        assert_eq!(lines(&root, "build.gradle", quiet), Vec::<usize>::new());
        // #230: `configure` on projects is still the project
        let src = "configure(subprojects.findAll { it.name != 'a' }) {\n    sourceCompatibility = 11\n}\nconfigure(rootProject) { archivesBaseName = 'x' }\n";
        assert_eq!(lines(&root, "build.gradle", src), vec![2, 4]);
        assert_eq!(
            lines(&root, "settings.gradle", "sourceCompatibility = 1"),
            Vec::<usize>::new()
        );
        // a gradle.properties up to the wrapper's directory that sets it makes it a project
        // property, which a build can set
        let src = "mainClassName = 'a.B'\nsourceCompatibility = 1.8\n";
        assert_eq!(lines(&sub, "build.gradle", src), vec![1, 2]);
        std::fs::write(sub.join("gradle.properties"), "# x\nmainClassName=a.C\n").unwrap();
        std::fs::write(root.join("gradle.properties"), "sourceCompatibility : 8\n").unwrap();
        assert_eq!(lines(&sub, "build.gradle", src), Vec::<usize>::new());
        std::fs::write(root.join("gradle.properties"), "sourceCompatibilityX=8\n").unwrap();
        assert_eq!(lines(&sub, "build.gradle", src), vec![2]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// #237: reads of the removed convention properties, each checked under Gradle 8.8,
    /// 9.0.0 and 9.9.1 (`libsDir` and `distsDir` under 4.10.3 and 7.6).
    #[test]
    fn removed_convention_reads() {
        let root = std::env::temp_dir().join(format!("groovyfmt-reads-{}", std::process::id()));
        let sub = root.join("sub");
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("settings.gradle"), "include 'sub'\n").unwrap();
        let props = root.join("gradle/wrapper/gradle-wrapper.properties");
        let wrapper = |v: &str| {
            std::fs::write(&props, format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n")).unwrap()
        };
        let lines = |dir: &Path, name: &str, src: &str| {
            rules(dir.join(name).to_str().unwrap(), src)
                .iter()
                .filter(|r| r.0 == "GradleRemovedApi")
                .map(|r| r.1)
                .collect::<Vec<_>>()
        };
        // jPOS's listJars task
        let src = "task listJars {\n    doLast {\n        println \"${project.libsDir}\"\n        println \"$project.libsDirName\"\n        println libsDirName\n        def d = project.libsDir\n    }\n}\nprintln sourceCompatibility\nallprojects { println \"$archivesBaseName\" }\ntasks.register('a') { doFirst { println mainClassName.size() } }\ntask('b') { println docsDir }\nsubprojects {\n    afterEvaluate { println project.webAppDir }\n}\nfiles(applicationDistribution)\n";
        wrapper("9.9.1");
        assert_eq!(
            lines(&root, "build.gradle", src),
            vec![3, 4, 5, 6, 9, 10, 11, 12, 14, 16]
        );
        // under Gradle 8 only libsDir, gone since 7.0; under 6, none
        wrapper("8.14.4");
        assert_eq!(lines(&root, "build.gradle", src), vec![3, 6]);
        wrapper("6.9");
        assert_eq!(lines(&root, "build.gradle", src), Vec::<usize>::new());
        wrapper("9.0.0");
        // the new forms, a typed task's or an extension's own property, another object's
        // property, a call, a named argument, a script method, a class, a closure parameter,
        // a local variable, an extra property, a settings script
        let quiet = "println base.libsDirectory.get()\njava { println docsDir }\ncompileJava { println sourceCompatibility }\ntask c(type: Ear) { doLast { println libDirName } }\ntasks.register('d', Jar) { println archivesBaseName }\njar { manifest { attributes 'Main-Class': mainClassName } }\nprintln foo.libsDir\nprintln rootProject.libsDir\ndeploymentDescriptor { }\nf(libsDir: 1)\ndef g() { libsDir }\nclass C { def h() { project.libsDir } }\nallprojects { p -> println libsDir }\ndef executableDir = 'b'\nprintln executableDir\next.webAppDirName = 'w'\nprintln webAppDirName\nconfigure(tasks.war) { println webAppDir }\ndef project = x\nprintln project.libsDir\n";
        assert_eq!(lines(&root, "build.gradle", quiet), Vec::<usize>::new());
        assert_eq!(
            lines(&root, "settings.gradle", "println sourceCompatibility\n"),
            Vec::<usize>::new()
        );
        // #248: a name the script tests with hasProperty is an extra property its subprojects
        // set; another name, an interpolated one, or a set of the tested name still counts
        let src = "subprojects {\n    tasks.register('e', JavaExec) {\n        if (!project.hasProperty('mainClassName')) { return }\n        mainClass = project.mainClassName\n    }\n}\nif (hasProperty(\"applicationName\")) { println applicationName }\nif (hasProperty('x')) { println project.executableDir }\nif (hasProperty(\"${n}Dir\")) { println project.docsDir }\nif (hasProperty('libsDirName')) { libsDirName = 'l' }\n";
        assert_eq!(lines(&root, "build.gradle", src), vec![8, 9, 10]);
        // an extra property a script above makes, or a gradle.properties value, can be read
        let src = "println project.archivesBaseName\nprintln mainClassName\nprintln libsDir\n";
        std::fs::write(sub.join("build.gradle"), src).unwrap();
        assert_eq!(lines(&sub, "build.gradle", src), vec![1, 2, 3]);
        std::fs::write(
            root.join("build.gradle"),
            "allprojects { ext.archivesBaseName = 'x' }\nsubprojects { ext { libsDir = 'd' } }\n",
        )
        .unwrap();
        std::fs::write(sub.join("gradle.properties"), "mainClassName=a.B\n").unwrap();
        assert_eq!(lines(&sub, "build.gradle", src), Vec::<usize>::new());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn removed_exec() {
        // #222: Project/Script exec and javaexec and JavaExec's `main`, removed in 9.0
        let root = std::env::temp_dir().join(format!("groovyfmt-exec-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let props = root.join("gradle/wrapper/gradle-wrapper.properties");
        let wrapper = |v: &str| {
            std::fs::write(&props, format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n")).unwrap()
        };
        let lines = |name: &str, src: &str| {
            rules(root.join(name).to_str().unwrap(), src)
                .iter()
                .filter(|r| r.0 == "GradleRemovedApi")
                .map(|r| r.1)
                .collect::<Vec<_>>()
        };
        let src = "def run() { exec { commandLine 'a' } }\ntasks.register('a') { doLast { exec { commandLine 'a' } } }\ntask b { doLast { javaexec { mainClass = 'X' } } }\ndef r = exec { ignoreExitValue = true }\nexec({ commandLine 'a' })\nproject.exec { commandLine 'a' }\nrootProject.javaexec { mainClass = 'X' }\ntasks.register('c', JavaExec) { main = 'X' }\ntask d(type: JavaExec) {\n    main 'X'\n}\ntasks.withType(JavaExec).configureEach { main = 'X' }\nbootRun { main = 'X' }\n";
        wrapper("9.0.0");
        assert_eq!(
            lines("build.gradle", src),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 13]
        );
        wrapper("9.9.1");
        assert_eq!(
            lines("build.gradle", src),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 13]
        );
        // under Gradle 8, or with no wrapper, quiet: they work there
        wrapper("8.8");
        assert_eq!(lines("build.gradle", src), Vec::<usize>::new());
        assert_eq!(rules("build.gradle", src), vec![]);
        wrapper("9.0.0");
        // ExecOperations, providers, an Exec task, a task's own exec(), a class's code, the
        // script's own exec method, other receivers, JavaExec's mainClass, a settings script
        let quiet = "interface I { @Inject ExecOperations getOps() }\ndef ops = objects.newInstance(I).ops\ntasks.register('a') { doLast { ops.exec { commandLine 'a' } } }\nexecOperations.javaexec { mainClass = 'X' }\nproviders.exec { commandLine 'a' }\ntasks.register('b', Exec) { commandLine 'a' }\nclass T extends Exec { void run() { exec() } }\nclass U { void run(p) { exec { } } }\nfoo.exec { }\ntasks.register('d') { doLast { injected.execOps.with { exec { commandLine 'a' } } } }\ntasks.register('c', JavaExec) { mainClass = 'X' }\nexec\n";
        assert_eq!(lines("build.gradle", quiet), Vec::<usize>::new());
        assert_eq!(
            lines(
                "build.gradle",
                "def exec(Closure c) { }\nexec { commandLine 'a' }\n"
            ),
            Vec::<usize>::new()
        );
        // #245: a settings or init script's too (Gradle 9.9.1: "Could not find method exec()")
        assert_eq!(
            lines("settings.gradle", "exec { commandLine 'a' }\n"),
            vec![1]
        );
        assert_eq!(
            lines("a.init.gradle", "exec { commandLine 'a' }\n"),
            vec![1]
        );
        assert_eq!(
            lines("settings.gradle", "providers.exec { commandLine 'a' }\n"),
            Vec::<usize>::new()
        );
        wrapper("8.14.4");
        assert_eq!(
            lines("settings.gradle", "exec { commandLine 'a' }\n"),
            Vec::<usize>::new()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// #245: more Gradle 9.0 removals, each checked to work under 8.14.4 and fail under
    /// 9.9.1, and each replacement to work under both.
    #[test]
    fn removed_in_9_scopes() {
        let root = std::env::temp_dir().join(format!("groovyfmt-245-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let props = root.join("gradle/wrapper/gradle-wrapper.properties");
        let wrapper = |v: &str| {
            std::fs::write(&props, format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n")).unwrap()
        };
        let lines = |src: &str| {
            rules(root.join("build.gradle").to_str().unwrap(), src)
                .iter()
                .filter(|r| r.0 == "GradleRemovedApi")
                .map(|r| r.1)
                .collect::<Vec<_>>()
        };
        // a) a convention name the java extension lacks, b) a plugin callback, g) an
        // archive task's block, d) a report's destination, e) a copy spec's modes, f) IDEA
        let src = "java { archivesBaseName = 'x' }
java {
    docsDirName = 'd'
}
pluginManager.withPlugin('java-base') { sourceCompatibility = 17 }
plugins.withId('java') { targetCompatibility = 17 }
plugins.withType(JavaPlugin) { archivesBaseName = 'x' }
pluginManager.withPlugin('java') { println sourceCompatibility }
jar { archivesBaseName = 'a' }
tasks.jar { archivesBaseName = 'a' }
jacocoTestReport {
    reports {
        html.destination = file('h')
        xml.destination file('x.xml')
        csv.destination(file('c'))
    }
}
test { reports.html.destination = file('t') }
test { reports { junitXml { destination = file('j') } } }
tasks.register('hook', Copy) {
    fileMode 0775
    dirMode = 0755
}
jar { from('a') { fileMode = 0755 } }
copy { into('b') { dirMode 0755 } }
idea { module { testSourceDirs += file('it') } }
idea {
    module {
        testResourceDirs += file('r')
    }
}
idea.module.testSourceDirs += file('it')
distributions { main { contents { from('bin') { fileMode = 0755 } } } }
";
        wrapper("9.9.1");
        assert_eq!(
            lines(src),
            vec![
                1, 3, 5, 6, 7, 8, 9, 10, 13, 14, 15, 18, 19, 21, 22, 24, 25, 26, 29, 32, 33
            ]
        );
        wrapper("9.0.0");
        assert_eq!(lines(src).len(), 21);
        // under Gradle 8, or with no wrapper, quiet: they work there
        wrapper("8.14.4");
        assert_eq!(lines(src), Vec::<usize>::new());
        assert_eq!(rules("build.gradle", src), vec![]);
        wrapper("9.9.1");
        // the replacements, the java extension's own compatibilities, a task's own
        // sourceCompatibility, a read of `destination` (write-only on 8.x as well), another
        // object's `destination` or `fileMode`, a lazy `into { }`, IDEA's other properties
        let quiet = "java { sourceCompatibility = 17; targetCompatibility = 17 }
pluginManager.withPlugin('java') { java { sourceCompatibility = 17 } }
tasks.withType(JavaCompile) { sourceCompatibility = '17' }
plugins.withType(JavaPlugin) { base { archivesName = 'x' } }
jar { archiveBaseName = 'a' }
tasks.named('compileJava') { targetCompatibility = 17 }
compileJava { sourceCompatibility = 17 }
jacocoTestReport { reports { html.outputLocation = file('h'); xml.required = true } }
println jacocoTestReport.reports.xml.destination
publication.destination = file('x')
html.destination = file('x')
ospackage { fileMode = 0644 }
ospackage { from('bin') { fileMode 0755 } }
from('x') { fileMode 0755 }
tasks.register('hook', Copy) {
    into { file('x') }
    filePermissions { unix(0775) }
    dirPermissions { unix(0755) }
}
idea { module { testSources.from(file('it')); sourceDirs += file('s') } }
module { testSourceDirs += file('x') }
";
        assert_eq!(lines(quiet), Vec::<usize>::new());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// #245: classes Groovy 4 moved out of groovy.util, checked to fail under 9.9.1 and
    /// work under 8.14.4, and the fixed imports to work under both.
    #[test]
    fn groovy4_moved_classes() {
        let root = std::env::temp_dir().join(format!("groovyfmt-g4-{}", std::process::id()));
        std::fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        let props = root.join("gradle/wrapper/gradle-wrapper.properties");
        let wrapper = |v: &str| {
            std::fs::write(&props, format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{v}-bin.zip\n")).unwrap()
        };
        let path = root.join("build.gradle");
        let path = path.to_str().unwrap();
        let lines = |src: &str| {
            rules(path, src)
                .iter()
                .filter(|r| r.0 == "GradleRemovedApi")
                .map(|r| r.1)
                .collect::<Vec<_>>()
        };
        let src = "// header
plugins { id 'java' }
def x = new XmlSlurper().parseText('<a/>')
XmlParser p = new XmlParser()
println new FileNameFinder().getFileNames('a', '*')
def y = new groovy.util.XmlSlurper()
def z = XmlSlurper.newInstance()
";
        wrapper("9.9.1");
        assert_eq!(lines(src), vec![3, 4, 4, 5, 6, 7]);
        assert_eq!(
            fixed(path, None, src),
            "// header
import groovy.xml.XmlSlurper
import groovy.xml.XmlParser
import groovy.ant.FileNameFinder
plugins { id 'java' }
def x = new XmlSlurper().parseText('<a/>')
XmlParser p = new XmlParser()
println new FileNameFinder().getFileNames('a', '*')
def y = new groovy.xml.XmlSlurper()
def z = XmlSlurper.newInstance()
"
        );
        // an import of the old package becomes the new one; a new import goes after the others
        let src = "import groovy.util.slurpersupport.GPathResult
import groovy.util.XmlSlurper
import java.io.File
GPathResult r = new XmlSlurper().parse(new File('x'))
println new XmlParser()";
        assert_eq!(lines(src), vec![1, 2, 5]);
        assert_eq!(
            fixed(path, None, src),
            "import groovy.xml.slurpersupport.GPathResult
import groovy.xml.XmlSlurper
import java.io.File
import groovy.xml.XmlParser
GPathResult r = new XmlSlurper().parse(new File('x'))
println new XmlParser()"
        );
        // the new imports, a star import, an alias, a class of the name, a field, other
        // packages' names
        let quiet = "import groovy.xml.XmlSlurper
import groovy.ant.*
import a.b.C as XmlParser
class Unrelated { }
println new XmlSlurper()
println new FileNameFinder()
println new XmlParser()
println foo.XmlSlurper
println new groovy.xml.XmlSlurper()
println new a.util.XmlSlurper()
";
        assert_eq!(lines(quiet), Vec::<usize>::new());
        assert_eq!(
            lines(
                "class XmlSlurper { def parseText(t) { t } }\nprintln new XmlSlurper().parseText('a')\n"
            ),
            Vec::<usize>::new()
        );
        // under Gradle 8 (Groovy 3), or with no wrapper, quiet
        wrapper("8.14.4");
        assert_eq!(lines("def x = new XmlSlurper()\n"), Vec::<usize>::new());
        assert_eq!(rules("build.gradle", "def x = new XmlSlurper()\n"), vec![]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn removed_configuration_reference_guarded_by_a_check() {
        // #222: WebViewUpgrade publish.gradle, never evaluated on 7+
        let src = "if (configurations.hasProperty(\"compile\")) {\n    classpath += configurations.compile\n}\nx = configurations.findByName('runtime') ? configurations.runtime : []\nif (configurations.hasProperty('compile')) {\n} else {\n    y = configurations.compile\n}\nif (configurations.hasProperty('runtime')) { z = configurations.compile }\n";
        assert_eq!(
            rules("build.gradle", src),
            vec![("GradleRemovedApi", 7), ("GradleRemovedApi", 9)]
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
        match crate::fix::fix(
            Path::new(path),
            src,
            compile,
            &crate::lint::Context::new(|_| None),
            wanted,
        ) {
            crate::fix::Fix::Fixed(t, _) => {
                assert_eq!(
                    crate::fix::fix(
                        Path::new(path),
                        &t,
                        compile,
                        &crate::lint::Context::new(|_| None),
                        wanted
                    ),
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
