//! Declarative Jenkins pipelines: the structure the pipeline-model-definition plugin checks
//! when a build starts, so a mistake shows up here instead of minutes into a run.
//!
//! Section lists follow https://www.jenkins.io/doc/book/pipeline/syntax/ and, where the two
//! differ, the plugin's own parser and validator (`ModelParser.groovy`, `ModelValidatorImpl.groovy`
//! in https://github.com/jenkinsci/pipeline-model-definition-plugin), which is what runs:
//! `libraries`, `failFast`, `input` and the `notBuilt` and `isRestartedRun` conditions are
//! accepted there but not all listed in the reference.
//!
//! Only a declarative pipeline is checked: a top-level `pipeline { }` in a file named
//! `Jenkinsfile`, `Jenkinsfile.*` or `*.jenkinsfile`, or in any file whose first statement it is.
//! Scripted pipelines (`node { }`) are never checked.

use super::{Violation, at, constant_value, has_interpolation};
use crate::text;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::Node;

const PIPELINE: &[&str] = &[
    "agent",
    "stages",
    "environment",
    "options",
    "parameters",
    "triggers",
    "tools",
    "post",
    "libraries",
];
const STAGE: &[&str] = &[
    "agent",
    "when",
    "steps",
    "post",
    "options",
    "input",
    "tools",
    "environment",
    "parallel",
    "matrix",
    "failFast",
    "stages",
];
const MATRIX: &[&str] = &[
    "axes",
    "excludes",
    "agent",
    "when",
    "post",
    "options",
    "input",
    "tools",
    "environment",
    "stages",
];
const POST: &[&str] = &[
    "always",
    "changed",
    "fixed",
    "regression",
    "aborted",
    "failure",
    "success",
    "unstable",
    "unsuccessful",
    "cleanup",
    "notBuilt",
];
/// Built-in `when` conditions, plus the `before*` flags that sit beside them.
const WHEN: &[&str] = &[
    "branch",
    "buildingTag",
    "changelog",
    "changeset",
    "changeRequest",
    "environment",
    "equals",
    "expression",
    "tag",
    "not",
    "allOf",
    "anyOf",
    "triggeredBy",
    "isRestartedRun",
    "beforeAgent",
    "beforeInput",
    "beforeOptions",
];
/// Job properties and pipeline-wide options: the validator rejects them in a stage's `options`
/// (StageOptions takes only wrappers and options whose descriptor allows stages).
const PIPELINE_ONLY_OPTIONS: &[&str] = &[
    "buildDiscarder",
    "disableConcurrentBuilds",
    "disableResume",
    "durabilityHint",
    "overrideIndexTriggers",
    "preserveStashes",
    "quietPeriod",
    "parallelsAlwaysFailFast",
    "disableRestartFromStage",
    "skipStagesAfterUnstable",
];
/// The ways a stage can say what it runs; it needs exactly one.
const RUNS: &[&str] = &["steps", "stages", "parallel", "matrix"];
/// Blocks nested deeper than this (`dir { dir { ... } }`, stages in stages, `allOf` in
/// `anyOf`) aren't checked: the checks recurse, and a worker thread's stack is 2 MiB.
/// Real pipelines nest a few levels.
const MAX_DEPTH: usize = 64;

/// A statement read as a method call: `name args`, `name(args)`, `name(args) { body }`,
/// `name { body }` or a bare `name`.
pub(super) struct Call<'a> {
    pub node: Node<'a>,
    pub name: &'a str,
    /// Arguments, not counting a trailing closure.
    pub args: Vec<Node<'a>>,
    /// The trailing closure.
    pub body: Option<Node<'a>>,
}

pub(super) fn call<'a>(n: Node<'a>, src: &'a str) -> Option<Call<'a>> {
    let mut args = Vec::new();
    let name = match n.kind() {
        "command_chain" => {
            let mut c = n.walk();
            args.extend(n.children_by_field_name("argument", &mut c));
            n.child_by_field_name("receiver")?
        }
        "expression_statement" => {
            let e = n.named_child(0)?;
            match e.kind() {
                "identifier" => {
                    args.extend(glued(n).and_then(|g| {
                        if g.kind() == "expression_statement" {
                            g.named_child(0)
                        } else {
                            Some(g)
                        }
                    }));
                    e
                }
                "method_invocation" => {
                    let f = e.child_by_field_name("function")?;
                    if f.kind() != "identifier" {
                        return None;
                    }
                    if let Some(a) = e.child_by_field_name("arguments") {
                        let mut c = a.walk();
                        args.extend(
                            a.named_children(&mut c)
                                .filter(|x| !x.kind().contains("comment")),
                        );
                    }
                    args.extend(e.child_by_field_name("closure"));
                    f
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    let body = if args.last().is_some_and(|a| a.kind() == "closure") {
        args.pop()
    } else {
        None
    };
    Some(Call {
        node: n,
        name: text(name, src),
        args,
        body,
    })
}

/// The grammar reads a command whose argument is an identifier or a method call, `agent any`
/// or `compile project(':a')`, as two statements, `agent` and `any`. This is the second one:
/// the statement right after a bare identifier, on the same line.
fn glued(n: Node) -> Option<Node> {
    let next = n.next_named_sibling()?;
    (n.kind() == "expression_statement"
        && n.named_child(0).is_some_and(|e| e.kind() == "identifier")
        && !next.kind().contains("comment")
        && next.start_position().row == n.end_position().row
        // `n` isn't itself the second half: in `agent none; steps { }`, `none` takes no argument
        && n.prev_named_sibling().and_then(glued) != Some(n))
    .then_some(next)
}

/// The statements of a closure, without comments, and without the second half of a
/// command the grammar split in two (`glued`).
pub(super) fn statements(body: Node) -> Vec<Node> {
    let mut c = body.walk();
    body.named_children(&mut c)
        .filter(|s| {
            !s.kind().contains("comment")
                && s.kind() != "closure_parameters"
                && s.prev_named_sibling().and_then(glued) != Some(*s)
        })
        .collect()
}

/// The first line of a statement, for messages.
fn head<'a>(n: Node, src: &'a str) -> &'a str {
    text(n, src).lines().next().unwrap_or("").trim()
}

pub(super) fn check(path: &Path, src: &str, root: Node, v: &mut Vec<Violation>) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let named =
        name == "Jenkinsfile" || name.starts_with("Jenkinsfile.") || name.ends_with(".jenkinsfile");
    let mut c = root.walk();
    let mut top = root.named_children(&mut c).filter(|n| {
        !n.kind().contains("comment")
            && !matches!(n.kind(), "import_declaration" | "package_declaration")
    });
    if named {
        top.filter(|n| n.kind() == "pipeline_statement")
            .for_each(|p| pipeline(src, p, v));
    } else if let Some(p) = top.next().filter(|n| n.kind() == "pipeline_statement") {
        pipeline(src, p, v);
    }
}

/// Reads the sections of a block: reports statements that aren't sections, names not in
/// `allowed` and repeats, and returns the sections.
fn sections<'a>(
    src: &'a str,
    body: Node<'a>,
    allowed: &[&str],
    what: &str,
    v: &mut Vec<Violation>,
) -> Vec<Call<'a>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for s in statements(body) {
        let Some(c) = call(s, src) else {
            v.push(at(
                s,
                "JenkinsStatementOutsideSection",
                format!(
                    "`{}` is not a section; {what} holds only {}. Code goes in a stage's steps {{ }} (inside script {{ }}) or outside pipeline {{ }}",
                    head(s, src),
                    allowed.join(", ")
                ),
            ));
            continue;
        };
        if !allowed.contains(&c.name) {
            v.push(at(
                s,
                "JenkinsUnknownSection",
                unknown(c.name, what, allowed),
            ));
            continue;
        }
        if !seen.insert(c.name) {
            v.push(at(
                s,
                "JenkinsConflictingSections",
                format!("`{}` appears more than once in this {what}", c.name),
            ));
        }
        out.push(c);
    }
    out
}

fn unknown(name: &str, what: &str, allowed: &[&str]) -> String {
    let hint = match name {
        "properties" | "jobProperties" | "wrappers" => {
            format!("`{name}` was replaced by options {{ }} in Declarative Pipeline 0.8")
        }
        "notifications" | "postBuild" => {
            format!("`{name}` was replaced by post {{ }} in Declarative Pipeline 0.8")
        }
        "stage" => "a stage goes inside stages { }".into(),
        "steps" | "parallel" | "matrix" => format!("`{name}` goes inside a stage"),
        "parameters" | "triggers" | "libraries" => {
            format!("`{name}` belongs directly in pipeline {{ }}, not in {what}")
        }
        _ => format!("`{name}` is not a section of {what}"),
    };
    format!("{hint}; {what} takes {}", allowed.join(", "))
}

fn pipeline(src: &str, p: Node, v: &mut Vec<Violation>) {
    let Some(body) = p.child_by_field_name("body") else {
        return;
    };
    let secs = sections(src, body, PIPELINE, "pipeline { }", v);
    for need in ["agent", "stages"] {
        if !secs.iter().any(|c| c.name == need) {
            let what = if need == "agent" {
                "`agent` (use `agent none` to pick agents per stage)"
            } else {
                "`stages`"
            };
            v.push(at(
                p,
                "JenkinsMissingSection",
                format!("pipeline {{ }} has no {what}, which Declarative Pipeline requires"),
            ));
        }
    }
    children(src, &secs, false, false, 0, v);
}

/// Checks the sections found in a pipeline, stage or matrix. `nested`: inside a `parallel`
/// or `matrix` (at any depth of `stages`), where another one is rejected.
fn children(
    src: &str,
    secs: &[Call],
    in_stage: bool,
    nested: bool,
    depth: usize,
    v: &mut Vec<Violation>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for c in secs {
        // the validator rejects these when empty
        if matches!(
            c.name,
            "steps"
                | "post"
                | "when"
                | "environment"
                | "options"
                | "parameters"
                | "triggers"
                | "libraries"
        ) && c.body.is_some_and(|b| statements(b).is_empty())
        {
            v.push(at(
                c.node,
                "JenkinsMissingSection",
                format!(
                    "`{} {{ }}` is empty, which Jenkins rejects; fill it in or remove it",
                    c.name
                ),
            ));
            continue;
        }
        match c.name {
            "agent" => agent(src, c, v),
            "options" => options(src, c, in_stage, v),
            "when" => {
                if let Some(b) = c.body {
                    when(src, b, depth + 1, v);
                }
            }
            "post" => {
                if let Some(b) = c.body {
                    for cond in sections(src, b, POST, "post { }", v) {
                        if let Some(b) = cond.body {
                            steps(src, b, depth + 1, v);
                        }
                    }
                }
            }
            "stages" | "parallel" => {
                if let Some(b) = c.body {
                    let nested = nested || c.name == "parallel";
                    stages(src, c.name, b, nested, depth + 1, v);
                }
            }
            "matrix" => {
                if let Some(b) = c.body {
                    let secs = sections(src, b, MATRIX, "matrix { }", v);
                    if !secs.iter().any(|s| s.name == "stages") {
                        v.push(at(
                            c.node,
                            "JenkinsMissingSection",
                            "matrix { } has no `stages`, which it requires".into(),
                        ));
                    }
                    // JenkinsMissingSection leaves a missing `axes` alone (it once read an
                    // ignored plugin test as valid); the validator requires it
                    // (ModelValidatorImpl: RequiredSection "axes").
                    if !secs.iter().any(|s| s.name == "axes") {
                        v.push(at(
                            c.node,
                            "JenkinsMatrixAxes",
                            "matrix { } has no `axes`, which it requires; add axes { axis { name 'X'; values 'a', 'b' } }".into(),
                        ));
                    }
                    matrix(src, &secs, v);
                    children(src, &secs, true, true, depth + 1, v);
                }
            }
            "input" => {
                if let Some(b) = c.body {
                    input(src, c.node, b, v);
                }
            }
            "environment" => {
                if let Some(b) = c.body {
                    environment(src, b, v);
                }
            }
            "triggers" => {
                if let Some(b) = c.body {
                    duplicate_names(src, b, "triggers", v);
                }
            }
            "tools" => {
                if let Some(b) = c.body {
                    tools(src, b, v);
                }
            }
            "steps" => {
                if let Some(b) = c.body {
                    steps(src, b, depth + 1, v);
                }
            }
            _ => {}
        }
    }
}

fn stages(src: &str, what: &str, body: Node, nested: bool, depth: usize, v: &mut Vec<Violation>) {
    let mut names = HashSet::new();
    let mut any = false;
    for s in statements(body) {
        let Some(c) = call(s, src) else {
            v.push(at(
                s,
                "JenkinsStatementOutsideSection",
                format!(
                    "`{}` is not a stage; {what} {{ }} holds only stage blocks. Code goes in a stage's steps {{ }} (inside script {{ }})",
                    head(s, src)
                ),
            ));
            continue;
        };
        if c.name != "stage" {
            v.push(at(
                s,
                "JenkinsUnknownSection",
                format!(
                    "`{}` is not a stage; {what} {{ }} holds only stage('name') {{ }} blocks (steps go in a stage's steps {{ }})",
                    c.name
                ),
            ));
            continue;
        }
        any = true;
        if let Some(n) = stage(src, &c, nested, depth, v)
            && !names.insert(n.clone())
        {
            v.push(at(
                s,
                "JenkinsDuplicateStageName",
                format!("Stage name '{n}' is used twice in this {what} {{ }}; Jenkins rejects duplicates"),
            ));
        }
    }
    if !any {
        v.push(at(
            body.parent().unwrap_or(body),
            "JenkinsMissingSection",
            format!(
                "{what} {{ }} has no stage; add at least one stage('name') {{ steps {{ ... }} }}"
            ),
        ));
    }
}

/// Checks a stage; returns its name when it's a valid literal.
fn stage(
    src: &str,
    c: &Call,
    nested: bool,
    depth: usize,
    v: &mut Vec<Violation>,
) -> Option<String> {
    let name = match c.args.first() {
        None => {
            v.push(at(
                c.node,
                "JenkinsStageName",
                "This stage has no name; write stage('Build') { ... }".into(),
            ));
            None
        }
        Some(&a)
            if (a.kind() == "string_literal" && !has_interpolation(a))
                || a.kind() == "identifier" =>
        {
            constant_value(a, src)
        }
        Some(&a) => {
            v.push(at(
                a,
                "JenkinsStageName",
                format!(
                    "Declarative Pipeline needs a stage name that is a plain string, not `{}`; GStrings and expressions are rejected",
                    head(a, src)
                ),
            ));
            None
        }
    };
    let body = c.body?;
    let secs = sections(src, body, STAGE, "stage { }", v);
    let runs: Vec<&Call> = secs.iter().filter(|s| RUNS.contains(&s.name)).collect();
    let label = name
        .as_deref()
        .map_or_else(String::new, |n| format!(" '{n}'"));
    if runs.is_empty() {
        v.push(at(
            c.node,
            "JenkinsMissingSection",
            format!(
                "Stage{label} has nothing to run; it needs one of steps, stages, parallel or matrix"
            ),
        ));
    } else if runs.len() > 1 && runs.iter().any(|r| r.name != runs[0].name) {
        v.push(at(
            runs[1].node,
            "JenkinsConflictingSections",
            format!(
                "Stage{label} has both `{}` and `{}`; a stage takes exactly one of steps, stages, parallel or matrix",
                runs[0].name, runs[1].name
            ),
        ));
    }
    // ModelValidatorImpl.validateElement(ModelASTStage): NoNestedWithinNestedStages first,
    // then (with one kind of run) AgentInNestedStages and ToolsInNestedStages
    let fan = runs
        .iter()
        .find(|r| matches!(r.name, "parallel" | "matrix"));
    if let Some(f) = fan {
        if nested {
            v.push(at(
                f.node,
                "JenkinsParallelNesting",
                format!(
                    "`{}` can't go in a stage that is itself inside parallel or matrix; Jenkins allows one level",
                    f.name
                ),
            ));
        } else if runs.iter().all(|r| r.name == f.name) {
            for s in secs.iter().filter(|s| matches!(s.name, "agent" | "tools")) {
                v.push(at(
                    s.node,
                    "JenkinsParallelNesting",
                    format!(
                        "`{}` is not allowed in stage{label}, which holds {}; put it in the stages inside",
                        s.name, f.name
                    ),
                ));
            }
        }
    }
    children(src, &secs, true, nested, depth, v);
    name
}

/// Plain Groovy statements directly in `steps { }` (or a post condition) are rejected: only
/// steps may go there, and code needs a `script { }` around it.
fn steps(src: &str, body: Node, depth: usize, v: &mut Vec<Violation>) {
    if depth > MAX_DEPTH {
        return;
    }
    let all = statements(body);
    for &s in &all {
        // steps the validator blocks in steps (BlockedStepsAndMethodCalls)
        let blocked = match call(s, src).map(|c| c.name) {
            Some("stage") => "a stage goes in stages { }, not in steps",
            Some("properties") => {
                "properties can't be used in steps; use options, parameters and triggers"
            }
            Some("parallel") if all.len() > 1 => {
                "parallel must be the only step in its steps { }; use parallel stages or wrap it in script { }"
            }
            _ => "",
        };
        if !blocked.is_empty() {
            v.push(at(s, "JenkinsUnknownSection", blocked.into()));
            continue;
        }
        // a block step's body holds steps too (`dir('x') { ... }`); `script` holds code
        if let Some(c) = call(s, src)
            && c.name != "script"
            && let Some(b) = c.body
        {
            steps(src, b, depth + 1, v);
            continue;
        }
        let what = match s.kind() {
            "if_statement" => "if",
            "for_statement" | "for_in_statement" => "for",
            "while_statement" | "do_while_statement" => "while",
            "try_statement" => "try",
            "local_variable_declaration" => "a variable declaration",
            "expression_statement" => match s.named_child(0).map(|e| e.kind()) {
                Some("assignment_expression") => "an assignment",
                Some("switch_expression") => "switch",
                _ => continue,
            },
            _ => continue,
        };
        v.push(at(
            s,
            "JenkinsScriptRequired",
            format!(
                "Declarative steps can't hold {what} (`{}`); wrap it in script {{ ... }}",
                head(s, src)
            ),
        ));
    }
}

fn when(src: &str, body: Node, depth: usize, v: &mut Vec<Violation>) {
    if depth > MAX_DEPTH {
        return;
    }
    for s in statements(body) {
        let Some(c) = call(s, src) else {
            v.push(at(
                s,
                "JenkinsStatementOutsideSection",
                format!(
                    "`{}` is not a when condition; put a Groovy test in expression {{ ... }}",
                    head(s, src)
                ),
            ));
            continue;
        };
        if !WHEN.contains(&c.name) {
            v.push(at(
                s,
                "JenkinsUnknownWhenCondition",
                format!(
                    "`{}` is not a built-in when condition (built in: {}); use expression {{ ... }} for other tests",
                    c.name,
                    WHEN.join(", ")
                ),
            ));
        } else if matches!(c.name, "allOf" | "anyOf" | "not") {
            // ModelValidatorImpl.validateElement(ModelASTWhenCondition)
            let n = c.body.map_or(0, |b| statements(b).len());
            let msg = if !c.args.is_empty() {
                format!("`{}` takes no arguments, only conditions in its block", c.name)
            } else if c.name == "not" && n != 1 {
                format!("`not` needs exactly one condition in its block, not {n}")
            } else if n == 0 {
                format!("`{}` needs at least one condition in its block", c.name)
            } else {
                String::new()
            };
            if !msg.is_empty() {
                v.push(at(s, "JenkinsWhenNesting", msg));
            }
            if let Some(b) = c.body {
                when(src, b, depth + 1, v);
            }
        } else if c.body.is_some() && c.name != "expression" {
            v.push(at(
                s,
                "JenkinsWhenNesting",
                format!(
                    "`{}` takes no block of conditions; only allOf, anyOf and not nest (and expression takes code)",
                    c.name
                ),
            ));
        }
    }
}

fn options(src: &str, c: &Call, in_stage: bool, v: &mut Vec<Violation>) {
    let Some(body) = c.body else { return };
    duplicate_names(src, body, "options", v);
    for s in statements(body) {
        let Some(o) = call(s, src) else { continue };
        let msg = match o.name {
            "parameters" | "triggers" | "pipelineTriggers" => format!(
                "`{}` doesn't go in options {{ }}; Declarative Pipeline has its own {} {{ }} section",
                o.name,
                if o.name == "parameters" {
                    "parameters"
                } else {
                    "triggers"
                }
            ),
            n if in_stage && PIPELINE_ONLY_OPTIONS.contains(&n) => format!(
                "`{n}` applies to the whole pipeline and is rejected in a stage's options; move it to the pipeline's options {{ }}"
            ),
            _ => continue,
        };
        v.push(at(s, "JenkinsMisplacedOption", msg));
    }
}

fn agent(src: &str, c: &Call, v: &mut Vec<Violation>) {
    let msg = match (c.body, c.args.as_slice()) {
        (Some(b), _) => {
            let s = statements(b);
            match s.as_slice() {
                [] => {
                    "agent { } is empty; name one agent type, as in agent { label 'linux' }".into()
                }
                [one] if call(*one, src).is_none() => format!(
                    "`{}` is not an agent type; write agent {{ label '...' }}, agent {{ docker {{ ... }} }} and so on",
                    head(*one, src)
                ),
                [_] => return,
                _ => "agent { } names more than one agent type; it takes exactly one".into(),
            }
        }
        (None, []) => {
            "agent needs a value: agent any, agent none, or a block such as agent { label 'linux' }"
                .into()
        }
        (None, [a]) if a.kind() == "identifier" => {
            let t = text(*a, src);
            if matches!(t, "any" | "none") {
                return;
            }
            format!(
                "`agent {t}` is not valid; only `any` and `none` stand alone, others take a block: agent {{ {t} ... }}"
            )
        }
        _ => return,
    };
    v.push(at(c.node, "JenkinsAgentForm", msg));
}

/// An option or trigger given twice in one block (DuplicateOptionName, DuplicateTriggerName).
fn duplicate_names(src: &str, body: Node, what: &str, v: &mut Vec<Violation>) {
    let mut seen = HashSet::new();
    for s in statements(body) {
        if let Some(c) = call(s, src)
            && !seen.insert(c.name)
        {
            v.push(at(
                s,
                "JenkinsDuplicateOption",
                format!("`{}` appears twice in this {what} {{ }}; Jenkins rejects a repeat", c.name),
            ));
        }
    }
}

/// Each entry of `tools { }` is a tool type and one version (ModelParser.parseTools).
fn tools(src: &str, body: Node, v: &mut Vec<Violation>) {
    for s in statements(body) {
        if call(s, src).is_some_and(|c| c.args.len() == 1 && c.body.is_none()) {
            continue;
        }
        v.push(at(
            s,
            "JenkinsToolVersion",
            format!(
                "`{}` is not a tool entry; tools {{ }} takes a tool type and one installation name, as in maven 'maven-3.9'",
                head(s, src)
            ),
        ));
    }
}

/// The fields of a stage's `input { }` (ModelParser.parseInput, and MissingInputMessage).
const INPUT: &[&str] = &[
    "message",
    "id",
    "ok",
    "submitter",
    "submitterParameter",
    "parameters",
];

fn input(src: &str, node: Node, body: Node, v: &mut Vec<Violation>) {
    let mut seen = HashSet::new();
    for s in statements(body) {
        let msg = match call(s, src) {
            None => format!("`{}` is not an input field", head(s, src)),
            Some(c) if !INPUT.contains(&c.name) => format!(
                "`{}` is not an input field; input takes {}",
                c.name,
                INPUT.join(", ")
            ),
            Some(c) if !seen.insert(c.name) => format!("`{}` appears twice in input {{ }}", c.name),
            Some(c) if c.args.len() + usize::from(c.body.is_some()) != 1 => {
                format!("input field `{}` takes exactly one value", c.name)
            }
            Some(_) => continue,
        };
        v.push(at(s, "JenkinsInputSection", msg));
    }
    if !seen.contains("message") {
        v.push(at(
            node,
            "JenkinsInputSection",
            "input { } has no `message`, which it requires".into(),
        ));
    }
}

/// What the validator takes as an environment variable or axis name
/// (Utils.validEnvIdentifier).
fn env_identifier(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Entries of `environment { }`: `KEY = value` with a valid, unique key, and a value that
/// is a literal, a string, a method call, or literals and strings joined with `+`
/// (ModelParser.parseEnvironment).
fn environment(src: &str, body: Node, v: &mut Vec<Violation>) {
    let mut seen = HashSet::new();
    for s in statements(body) {
        let a = s
            .named_child(0)
            .filter(|a| s.kind() == "expression_statement" && a.kind() == "assignment_expression")
            .filter(|a| {
                a.child_by_field_name("operator")
                    .is_some_and(|o| o.kind() == "=")
            });
        let (Some(key), Some(value)) = (
            a.and_then(|a| a.child_by_field_name("left")),
            a.and_then(|a| a.child_by_field_name("right")),
        ) else {
            v.push(at(
                s,
                "JenkinsEnvironmentEntry",
                format!(
                    "`{}` is not an environment entry; environment {{ }} takes only KEY = value lines",
                    head(s, src)
                ),
            ));
            continue;
        };
        let k = text(key, src);
        let msg = if key.kind() != "identifier" || !env_identifier(k) {
            format!(
                "`{k}` is not a valid environment variable name; use letters, digits and _, not starting with a digit"
            )
        } else if !seen.insert(k) {
            format!("`{k}` is set twice in this environment {{ }}; Jenkins rejects a repeat")
        } else if !env_value(value) {
            format!(
                "The value of `{k}` must be a quoted string, a literal or a method call (`{}` is none of these); write \"${{...}}\" to use an expression",
                head(value, src)
            )
        } else {
            continue;
        };
        v.push(at(s, "JenkinsEnvironmentEntry", msg));
    }
}

fn env_value(n: Node) -> bool {
    match n.kind() {
        "string_literal" | "number_literal" | "boolean_literal" | "null_literal"
        | "method_invocation" | "command_chain" => true,
        // `-1`: whether Groovy reads it as a constant isn't certain, so it is let through
        "unary_expression" => true,
        "parenthesized_expression" => n.named_child(0).is_some_and(env_value),
        "binary_expression" => {
            n.child_by_field_name("operator")
                .is_some_and(|o| o.kind() == "+")
                && n.child_by_field_name("left").is_some_and(concat_part)
                && n.child_by_field_name("right").is_some_and(|r| {
                    matches!(
                        r.kind(),
                        "string_literal" | "number_literal" | "boolean_literal" | "null_literal"
                    )
                })
        }
        _ => false,
    }
}

/// The left side of a `+` in an environment value: a literal, or another `+` of them
/// (ModelParser.envValueForStringConcat).
fn concat_part(n: Node) -> bool {
    match n.kind() {
        "string_literal" | "number_literal" | "boolean_literal" | "null_literal" => true,
        "binary_expression" => env_value(n),
        _ => false,
    }
}

/// `axes` and `excludes` of a matrix (ModelParser.parseMatrix*, ModelValidatorImpl for
/// ModelASTAxisContainer, ModelASTAxis, ModelASTExcludes, ModelASTExclude).
fn matrix(src: &str, secs: &[Call], v: &mut Vec<Violation>) {
    let mut push = |n: Node, m: String| v.push(at(n, "JenkinsMatrixAxes", m));
    for sec in secs.iter().filter(|s| matches!(s.name, "axes" | "excludes")) {
        let Some(body) = sec.body else { continue };
        let item = if sec.name == "axes" {
            "axis"
        } else {
            "exclude"
        };
        let items: Vec<Call> = statements(body)
            .into_iter()
            .filter_map(|s| {
                let c = call(s, src).filter(|c| c.name == item && c.body.is_some());
                if c.is_none() {
                    push(
                        s,
                        format!("`{}` is not an {item} block; {} {{ }} holds only {item} {{ }} blocks", head(s, src), sec.name),
                    );
                }
                c
            })
            .collect();
        if items.is_empty() {
            push(sec.node, format!("{} {{ }} has no {item}", sec.name));
        }
        // the axes of `axes`, or of each `exclude`
        let groups: Vec<Vec<Call>> = if sec.name == "axes" {
            vec![items]
        } else {
            items
                .into_iter()
                .filter_map(|e| {
                    let b = e.body?;
                    let axes: Vec<Call> = statements(b)
                        .into_iter()
                        .filter_map(|s| {
                            let c = call(s, src).filter(|c| c.name == "axis" && c.body.is_some());
                            if c.is_none() {
                                push(
                                    s,
                                    format!("`{}` is not an axis block; exclude {{ }} holds only axis {{ }} blocks", head(s, src)),
                                );
                            }
                            c
                        })
                        .collect();
                    if axes.is_empty() {
                        push(e.node, "exclude { } has no axis".into());
                    }
                    Some(axes)
                })
                .collect()
        };
        for axes in groups {
            let mut names = HashSet::new();
            for a in axes {
                axis(src, &a, sec.name == "excludes", &mut names, &mut push);
            }
        }
    }
}

fn axis<'a>(
    src: &'a str,
    a: &Call<'a>,
    exclude: bool,
    names: &mut HashSet<String>,
    push: &mut impl FnMut(Node, String),
) {
    let Some(body) = a.body else { return };
    let keys: &[&str] = if exclude {
        &["name", "values", "notValues"]
    } else {
        &["name", "values"]
    };
    let mut seen = HashSet::new();
    let mut values = HashSet::new();
    for s in statements(body) {
        let Some(c) = call(s, src).filter(|c| keys.contains(&c.name)) else {
            push(
                s,
                format!("`{}` is not part of an axis; an axis takes {}", head(s, src), keys.join(", ")),
            );
            continue;
        };
        if !seen.insert(c.name) {
            push(s, format!("`{}` appears twice in this axis", c.name));
            continue;
        }
        if c.name == "name" {
            let lit = c
                .args
                .first()
                .filter(|a| c.args.len() == 1 && a.kind() == "string_literal")
                .and_then(|a| constant_value(*a, src));
            match lit {
                None => push(s, "An axis name must be one plain string, as in name 'OS'".into()),
                Some(n) if !exclude && !env_identifier(&n) => push(
                    s,
                    format!("Axis name '{n}' is not a valid variable name; use letters, digits and _, not starting with a digit"),
                ),
                // the validator reports a repeat only in the axes of one block
                Some(n) if !names.insert(n.clone()) => {
                    push(s, format!("Axis name '{n}' is used twice"))
                }
                Some(_) => {}
            }
        } else {
            if c.args.is_empty() {
                push(s, format!("`{}` lists no values", c.name));
            }
            // an exclude's values aren't validated beyond parsing
            for &x in c.args.iter().filter(|_| !exclude) {
                match constant_value(x, src).filter(|_| x.kind() != "identifier") {
                    None => push(
                        x,
                        format!("Axis value `{}` must be a plain string or number, not an expression or interpolated string", head(x, src)),
                    ),
                    Some(val) if !values.insert((x.kind() == "string_literal", val.clone())) => {
                        push(x, format!("Axis value '{val}' is listed twice"))
                    }
                    Some(_) => {}
                }
            }
        }
    }
    if seen.contains("values") && seen.contains("notValues") {
        push(a.node, "An exclude axis takes values or notValues, not both".into());
    }
    if !exclude {
        for need in ["name", "values"] {
            if !seen.contains(need) {
                push(a.node, format!("This axis has no `{need}`, which it requires"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::lint::lint_file;
    use std::path::Path;

    fn rules(src: &str) -> Vec<(&'static str, usize)> {
        lint_file(Path::new("Jenkinsfile"), src)
            .expect("parses")
            .into_iter()
            .filter(|v| v.rule.starts_with("Jenkins"))
            .map(|v| (v.rule, v.line))
            .collect()
    }

    /// A pipeline with `body` as the steps of one stage, starting on line 6.
    fn in_steps(body: &str) -> String {
        format!(
            "pipeline {{\n agent any\n stages {{\n  stage('a') {{\n   steps {{\n{body}\n   }}\n  }}\n }}\n}}\n"
        )
    }

    const VALID: &str = "pipeline {
    agent { docker { image 'gradle:8' } }
    options { timeout(time: 1, unit: 'HOURS'); timestamps() }
    parameters { string(name: 'X', defaultValue: '') }
    environment { A = 'b' }
    stages {
        stage('Build') {
            when { allOf { branch 'main'; not { buildingTag() } }; beforeAgent true }
            options { skipDefaultCheckout true; retry(2) }
            steps {
                sh 'make'
                dir('sub') { sh './gradlew build' }
                script {
                    if (params.X) { def y = 1 }
                }
            }
            post { failure { echo 'failed' } }
        }
        stage(Test) {
            parallel {
                stage('unit') { agent none; steps { echo 'u' } }
                stage('it') { steps { echo 'i' } }
            }
        }
        stage('Matrix') {
            matrix {
                axes { axis { name 'OS'; values 'linux', 'mac' } }
                stages { stage('m') { steps { echo 'm' } } }
            }
        }
        stage('Nested') { stages { stage('n') { steps { parallel(a: { echo 'a' }, b: { echo 'b' }) } } } }
    }
    post { always { echo 'done' }; notBuilt { echo 'x' } }
}
";

    #[test]
    fn valid_pipeline_is_clean() {
        assert_eq!(rules(VALID), vec![]);
        // as the first statement of any file, after comments and imports
        let src = format!("// header\nimport a.B\n{VALID}");
        let v = lint_file(Path::new("ci.groovy"), &src).unwrap();
        assert!(v.iter().all(|v| !v.rule.starts_with("Jenkins")), "{v:?}");
    }

    #[test]
    fn only_declarative_pipelines_are_checked() {
        let bad = "pipeline {\n stages { }\n}\n";
        // a scripted pipeline, or a pipeline that isn't the first statement of a .groovy file
        for (name, src) in [
            (
                "Jenkinsfile",
                "node {\n stage('x') { sh 'make' }\n}\n".to_string(),
            ),
            ("x.groovy", format!("def x = 1\n{bad}")),
        ] {
            let v = lint_file(Path::new(name), &src).unwrap();
            assert!(v.iter().all(|v| !v.rule.starts_with("Jenkins")), "{name}");
        }
        for name in ["Jenkinsfile", "Jenkinsfile.release", "deploy.jenkinsfile"] {
            let src = format!("def x = 1\n{bad}");
            let v = lint_file(Path::new(name), &src).unwrap();
            assert!(
                v.iter().any(|v| v.rule == "JenkinsMissingSection"),
                "{name}"
            );
        }
    }

    #[test]
    fn missing_sections() {
        assert_eq!(
            rules("pipeline {\n stages {\n }\n}\n"),
            vec![("JenkinsMissingSection", 1), ("JenkinsMissingSection", 2)]
        );
        assert_eq!(
            rules(
                "pipeline {\n agent any\n stages {\n  stage('a') {\n   agent any\n  }\n  stage('b') { steps { } }\n }\n}\n"
            ),
            vec![("JenkinsMissingSection", 4), ("JenkinsMissingSection", 7)]
        );
        assert_eq!(
            rules("pipeline {\n agent any\n}\n"),
            vec![("JenkinsMissingSection", 1)]
        );
    }

    #[test]
    fn conflicting_sections() {
        assert_eq!(
            rules(
                "pipeline {\n agent any\n agent none\n stages {\n  stage('a') {\n   steps { echo 'a' }\n   stages { stage('b') { steps { echo 'b' } } }\n  }\n }\n}\n"
            ),
            vec![
                ("JenkinsConflictingSections", 3),
                ("JenkinsConflictingSections", 7)
            ]
        );
    }

    #[test]
    fn unknown_and_misplaced_sections() {
        let src = "pipeline {\n agent any\n properties { }\n steps { echo 'x' }\n stages {\n  stage('a') {\n   parameters { string(name: 'x') }\n   steps { echo 'a' }\n  }\n  echo 'b'\n }\n post { sucess { echo 'c' } }\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("JenkinsUnknownSection", 3),
                ("JenkinsUnknownSection", 4),
                ("JenkinsUnknownSection", 7),
                ("JenkinsUnknownSection", 10),
                ("JenkinsUnknownSection", 12),
            ]
        );
        // steps the validator blocks inside steps
        assert_eq!(
            rules(&in_steps(
                "stage('x') { }\nproperties([])\necho 'a'\nparallel(a: { echo 'a' })"
            )),
            vec![
                ("JenkinsUnknownSection", 6),
                ("JenkinsUnknownSection", 7),
                ("JenkinsUnknownSection", 9)
            ]
        );
        // `parallel` as the only step is allowed
        assert_eq!(rules(&in_steps("parallel(a: { echo 'a' })")), vec![]);
    }

    #[test]
    fn statements_outside_sections() {
        let src = "pipeline {\n agent any\n def x = 1\n stages {\n  x = 2\n  stage('a') {\n   if (x) { }\n   steps { echo 'a' }\n  }\n }\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("JenkinsStatementOutsideSection", 3),
                ("JenkinsStatementOutsideSection", 5),
                ("JenkinsStatementOutsideSection", 7),
            ]
        );
    }

    #[test]
    fn code_in_steps_needs_script() {
        let src = in_steps(
            "if (a) { echo 'a' }\nfor (x in xs) { echo x }\ndef y = 1\ncurrentBuild.result = 'FAILURE'\ntry { echo 'a' } finally { echo 'b' }\ndir('d') { while (true) { } }\nscript { if (a) { def z = 2 } }",
        );
        assert_eq!(
            rules(&src),
            vec![
                ("JenkinsScriptRequired", 6),
                ("JenkinsScriptRequired", 7),
                ("JenkinsScriptRequired", 8),
                ("JenkinsScriptRequired", 9),
                ("JenkinsScriptRequired", 10),
                ("JenkinsScriptRequired", 11),
            ]
        );
    }

    #[test]
    fn when_conditions() {
        let src = "pipeline {\n agent any\n stages {\n  stage('a') {\n   when {\n    anyOf { branch 'main'; onBranch 'x' }\n    isRestartedRun()\n   }\n   steps { echo 'a' }\n  }\n }\n}\n";
        assert_eq!(rules(src), vec![("JenkinsUnknownWhenCondition", 6)]);
    }

    #[test]
    fn misplaced_options() {
        let src = "pipeline {\n agent any\n options {\n  buildDiscarder(logRotator(numToKeepStr: '5'))\n  parameters([string(name: 'x')])\n }\n stages {\n  stage('a') {\n   options { disableConcurrentBuilds(); timeout(5); checkoutToSubdirectory('x') }\n   steps { echo 'a' }\n  }\n }\n}\n";
        assert_eq!(
            rules(src),
            vec![("JenkinsMisplacedOption", 5), ("JenkinsMisplacedOption", 9)]
        );
    }

    #[test]
    fn stage_names() {
        let src = "pipeline {\n agent any\n stages {\n  stage('a') { steps { echo 1 } }\n  stage(\"a\") { steps { echo 1 } }\n  stage(\"b ${x}\") { steps { echo 1 } }\n  stage { steps { echo 1 } }\n  stage(name: 'c') { steps { echo 1 } }\n  stage('d') { parallel { stage('e') { steps { echo 1 } }\n stage('e') { steps { echo 1 } } } }\n }\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("JenkinsDuplicateStageName", 5),
                ("JenkinsStageName", 6),
                ("JenkinsStageName", 7),
                ("JenkinsStageName", 8),
                ("JenkinsDuplicateStageName", 10),
            ]
        );
    }

    #[test]
    fn agent_forms() {
        let src = "pipeline {\n agent anything\n stages {\n  stage('a') {\n   agent { }\n   steps { echo 1 }\n  }\n  stage('b') {\n   agent { label 'x'; docker 'y' }\n   steps { echo 1 }\n  }\n  stage('c') {\n   agent\n   steps { echo 1 }\n  }\n  stage('d') {\n   agent { kubernetes { yaml 'x' } }\n   steps { echo 1 }\n  }\n }\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("JenkinsAgentForm", 2),
                ("JenkinsAgentForm", 5),
                ("JenkinsAgentForm", 9),
                ("JenkinsAgentForm", 13),
            ]
        );
    }

    /// A pipeline with `body` as its stages, starting on line 4.
    fn in_stages(body: &str) -> String {
        format!("pipeline {{\n agent none\n stages {{\n{body}\n }}\n}}\n")
    }

    #[test]
    fn parallel_nesting() {
        // the plugin's errors/parallelStagesDeepNesting, matrixStagesAgentTools and
        // parallelStagesGroupsDeepNesting
        let src = in_stages(
            "stage('a') {\n parallel {\n  stage('b') {\n   parallel { stage('c') { steps { echo 1 } } }\n  }\n }\n}\nstage('d') {\n agent any\n tools { maven 'm' }\n matrix {\n  axes { axis { name 'X'; values 'a' } }\n  stages { stage('e') { stages { stage('f') { matrix {\n   axes { axis { name 'Y'; values 'b' } }\n   stages { stage('g') { steps { echo 1 } } }\n  } } } } }\n }\n}",
        );
        assert_eq!(
            rules(&src),
            vec![
                ("JenkinsParallelNesting", 7),
                ("JenkinsParallelNesting", 12),
                ("JenkinsParallelNesting", 13),
                ("JenkinsParallelNesting", 16),
            ]
        );
        // one level, agent in the branches, and stages in stages are fine
        let src = in_stages(
            "stage('a') {\n parallel {\n  stage('b') { agent any; stages { stage('c') { steps { echo 1 } } } }\n }\n}\nstage('d') { agent any; stages { stage('e') { parallel { stage('f') { steps { echo 1 } } } } } }",
        );
        assert_eq!(rules(&src), vec![]);
    }

    #[test]
    fn when_nesting() {
        let when = |w: &str| {
            in_stages(&format!(
                "stage('a') {{\n when {{\n{w}\n }}\n steps {{ echo 1 }}\n}}"
            ))
        };
        // errors/nestedWhenWithArgs, invalidWhenWithChildren, whenNestedChildrenInvalid,
        // when/conditions/allOfEmpty
        assert_eq!(
            rules(&when(
                "allOf(thing: 'x') { branch 'a' }\nbranch(compare: 'x') { not { branch 'b' } }\nanyOf { }\nnot { branch 'a'; branch 'b' }\nnot { }"
            )),
            vec![
                ("JenkinsWhenNesting", 6),
                ("JenkinsWhenNesting", 7),
                ("JenkinsWhenNesting", 8),
                ("JenkinsWhenNesting", 9),
                ("JenkinsWhenNesting", 10),
            ]
        );
        assert_eq!(
            rules(&when(
                "allOf { branch 'a'; anyOf { tag 'v*'; expression { return x } } }\nnot { changeRequest() }\nbeforeAgent true"
            )),
            vec![]
        );
    }

    #[test]
    fn matrix_axes() {
        let matrix = |m: &str| {
            in_stages(&format!(
                "stage('a') {{\n matrix {{\n{m}\n  stages {{ stage('b') {{ steps {{ echo 1 }} }} }}\n }}\n}}"
            ))
        };
        assert_eq!(rules(&matrix("")), vec![("JenkinsMatrixAxes", 5)]);
        assert_eq!(rules(&matrix("axes { }")), vec![("JenkinsMatrixAxes", 6)]);
        let src = matrix(
            "axes {\n axis { name 'OS'; values 'linux', 'mac', 'linux' }\n axis { name 'OS'; values 1, '1' }\n axis { name '1X'; values \"${v}\" }\n axis { values 'a' }\n axis { name 'Y' }\n}\nexcludes { }",
        );
        assert_eq!(
            rules(&src),
            vec![
                ("JenkinsMatrixAxes", 7),
                ("JenkinsMatrixAxes", 8),
                ("JenkinsMatrixAxes", 9),
                ("JenkinsMatrixAxes", 9),
                ("JenkinsMatrixAxes", 10),
                ("JenkinsMatrixAxes", 11),
                ("JenkinsMatrixAxes", 13),
            ]
        );
        let src = matrix(
            "axes { axis { name 'OS'; values 'linux', 'mac' }; axis { name 'B'; values 'x' } }\nexcludes {\n exclude { axis { name 'OS'; values 'linux'; notValues 'mac' } }\n exclude { }\n exclude { axis { name 'OS'; values 'mac' }; axis { name 'B'; notValues 'x' } }\n}",
        );
        assert_eq!(
            rules(&src),
            vec![("JenkinsMatrixAxes", 8), ("JenkinsMatrixAxes", 9)]
        );
    }

    #[test]
    fn input_section() {
        let input = |i: &str| {
            in_stages(&format!(
                "stage('a') {{\n input {{\n{i}\n }}\n steps {{ echo 1 }}\n}}"
            ))
        };
        assert_eq!(
            rules(&input("ok 'Go'\nbanana 'x'\nok 'Again'")),
            vec![
                ("JenkinsInputSection", 5),
                ("JenkinsInputSection", 7),
                ("JenkinsInputSection", 8),
            ]
        );
        assert_eq!(
            rules(&input(
                "message 'Deploy?'\nid 'x'\nok 'Yes'\nsubmitter 'admin'\nsubmitterParameter 'WHO'\nparameters { string(name: 'P') }"
            )),
            vec![]
        );
    }

    #[test]
    fn environment_entries() {
        let env = |e: &str| {
            format!(
                "pipeline {{\n agent any\n environment {{\n{e}\n }}\n stages {{ stage('a') {{ steps {{ echo 1 }} }} }}\n}}\n"
            )
        };
        // errors/duplicateEnvironment, envIdentifierHyphens, additionalInvalidExpressionsInEnvironment
        let src = env(
            "A = env.B\nC = params.D\nE = x ? 'a' : 'b'\nF = 1 + credentials('x')\nG = 'a' - 'b'\nA = 'again'\nenv.H = 'x'\necho 'hi'\nI += 'x'",
        );
        assert_eq!(
            rules(&src),
            (4..=12).map(|l| ("JenkinsEnvironmentEntry", l)).collect::<Vec<_>>()
        );
        // environment/nonLiteralEnvironment, readFileInEnv and multilineEnvironment
        let src = env(
            "A = 'b'\nB = \"${env.X}\"\nC = credentials('c')\nD = currentBuild.getNumber()\nE = readFile 'Version'\nF = 'a' + \"b\" + 1\nG = 1\nH = true\nI = '''x\ny'''\n_J = ('k')\nK = -1",
        );
        assert_eq!(rules(&src), vec![]);
    }

    #[test]
    fn duplicate_options_and_triggers() {
        let src = "pipeline {\n agent any\n options {\n  timeout(time: 1, unit: 'HOURS')\n  skipDefaultCheckout()\n  timeout(time: 2, unit: 'HOURS')\n }\n triggers {\n  cron('H * * * *')\n  cron('H 2 * * *')\n  pollSCM('H * * * *')\n }\n stages { stage('a') { steps { echo 1 } } }\n}\n";
        assert_eq!(
            rules(src),
            vec![("JenkinsDuplicateOption", 6), ("JenkinsDuplicateOption", 10)]
        );
    }

    #[test]
    fn tool_versions() {
        let src = "pipeline {\n agent any\n tools {\n  maven\n  jdk 'a', 'b'\n  gradle 'g8'\n }\n stages { stage('a') { steps { echo 1 } } }\n}\n";
        assert_eq!(
            rules(src),
            vec![("JenkinsToolVersion", 4), ("JenkinsToolVersion", 5)]
        );
    }

    #[test]
    fn deep_nesting_is_checked_to_a_limit_without_overflowing() {
        // a mistake n blocks down: found at 3, not checked at 5,000, and no stack overflow
        let nest = |open: &str, inner: &str, close: &str, n| {
            format!("{}{inner}{}", open.repeat(n), close.repeat(n))
        };
        let stage = |body: &str| {
            format!("pipeline {{\n agent any\n stages {{\n  stage('a') {{ {body} }}\n }}\n}}\n")
        };
        for (n, found) in [(3, true), (5_000, false)] {
            for (src, rule) in [
                (
                    in_steps(&nest("dir('x') { ", "def y = 1", " }", n)),
                    "JenkinsScriptRequired",
                ),
                (
                    stage(&nest("stages { stage('s') { ", "x = 1", " } }", n)),
                    "JenkinsStatementOutsideSection",
                ),
                (
                    stage(&format!(
                        "when {{ {} }}\n   steps {{ echo 'a' }}",
                        nest("allOf { ", "onBranch 'x'", " }", n)
                    )),
                    "JenkinsUnknownWhenCondition",
                ),
            ] {
                let v = rules(&src);
                assert_eq!(v.iter().any(|r| r.0 == rule), found, "{rule} {n}: {v:?}");
            }
        }
    }
}
