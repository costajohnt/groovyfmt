//! `--import-codenarc`: reads CodeNarc configurations (ruleset XML, the Groovy ruleset DSL,
//! `codenarc.properties`) and npm-groovy-lint's `.groovylintrc.json`, and works out the
//! `[lint]` table that reproduces them, with a report of what groovyfmt can't carry over.
//!
//! Nothing is executed and nothing is fetched. The DSL is code, so it is read with
//! groovyfmt's own parser and only its declarative forms are understood; anything else is
//! reported, not guessed at. The XML reader knows only CodeNarc's ruleset schema and rejects
//! DTDs, entities other than XML's own, and anything it doesn't know. The rulesets CodeNarc
//! bundles and npm-groovy-lint's presets come from `codenarc_data.rs`, generated from their
//! sources by `scripts/codenarc-data.py`.

use crate::codenarc_data::{CODENARC_VERSION, NPM_GROOVY_LINT_VERSION, PRESETS, RULES, RULESETS};
use crate::{display_path, text};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tree_sitter::Node;

/// Larger inputs are refused: real configurations are a few kilobytes.
pub const MAX_BYTES: u64 = 1 << 20;
/// Nesting deeper than this is refused (CodeNarc's schema nests four levels).
const MAX_XML_DEPTH: usize = 8;
/// How deep rulesets may reference other ruleset files.
const MAX_REFS: usize = 8;
const NAMESPACE: &str = "http://codenarc.org/ruleset/1.0";

/// The CodeNarc formatting rules groovyfmt's formatter covers, and how. Only rules whose
/// violations formatting removes are here (checked against the formatter, not assumed from
/// the names); the others count as rules groovyfmt has no equivalent for.
const FORMATTER: &[(&str, &str)] = &[
    (
        "ConsecutiveBlankLines",
        "runs of blank lines always collapse to one",
    ),
    (
        "FileEndsWithoutNewline",
        "files always end with one newline",
    ),
    (
        "Indentation",
        "every line is re-indented (indent, continuation_indent, use_tabs)",
    ),
    (
        "TrailingWhitespace",
        "trailing whitespace is always removed",
    ),
    ("SpaceAfterCatch", SPACING),
    ("SpaceAfterClosingBrace", SPACING),
    ("SpaceAfterComma", SPACING),
    ("SpaceAfterFor", SPACING),
    ("SpaceAfterIf", SPACING),
    ("SpaceAfterMethodCallName", SPACING),
    ("SpaceAfterMethodDeclarationName", SPACING),
    ("SpaceAfterOpeningBrace", SPACING),
    ("SpaceAfterSwitch", SPACING),
    ("SpaceAfterWhile", SPACING),
    ("SpaceAroundClosureArrow", SPACING),
    ("SpaceAroundOperator", SPACING),
    ("SpaceBeforeClosingBrace", SPACING),
    ("SpaceBeforeOpeningBrace", SPACING),
    ("SpaceInsideParentheses", SPACING),
];
const SPACING: &str = "normalize_spacing = true";

/// A rule as CodeNarc would hold it after reading the configuration: CodeNarc runs each
/// instance, so a rule is on when any instance of it is.
struct Inst {
    name: String,
    on: bool,
    /// Where it came from or was last switched, for `-v`.
    why: String,
}

/// A property value from the source: a literal, or code that isn't evaluated.
enum Val {
    Lit(String),
    Code(String),
}

impl Val {
    fn text(&self) -> String {
        match self {
            Val::Lit(s) | Val::Code(s) => s.clone(),
        }
    }
    /// `enabled`'s value, if it is a literal boolean (CodeNarc reads "true" and "false").
    fn bool(&self) -> Option<bool> {
        match self {
            Val::Lit(s) if s.eq_ignore_ascii_case("true") => Some(true),
            Val::Lit(s) if s.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        }
    }
}

/// The settings a ruleset reference applies to its rules: CodeNarc applies the configs,
/// then keeps the rules matching an include (all if none) and no exclude.
#[derive(Default)]
struct Filter {
    includes: Vec<String>,
    excludes: Vec<(String, String)>,
    /// rule, property, value, where
    configs: Vec<(String, String, Val, String)>,
}

pub struct Import {
    insts: Vec<Inst>,
    /// Some input defines a set of rules; `codenarc.properties` alone only adjusts one.
    defines_set: bool,
    /// Rule, property and value of each property set in the source, other than `enabled`.
    props: BTreeSet<(String, String, String)>,
    /// What couldn't be read or interpreted, as `file:line: what`.
    notes: Vec<String>,
    /// Rulesets read, for `-v`.
    rulesets: Vec<String>,
    /// The input files as named, for the header.
    inputs: Vec<String>,
    /// The directory of the input being read: referenced files are read only inside it.
    root: PathBuf,
    /// Files being read, outermost first, so a reference cycle stops.
    reading: Vec<PathBuf>,
}

/// Reads each file in turn. An error is a file that can't be read or isn't a configuration
/// this understands; what can't be carried over from one that is goes in the report.
pub fn import(files: &[PathBuf]) -> Result<Import, String> {
    let mut im = Import {
        insts: Vec::new(),
        defines_set: false,
        props: BTreeSet::new(),
        notes: Vec::new(),
        rulesets: Vec::new(),
        inputs: files.iter().map(|f| display_path(f)).collect(),
        root: PathBuf::new(),
        reading: Vec::new(),
    };
    // rulesets first: codenarc.properties adjusts the rules they define, as in CodeNarc
    let (props, sets): (Vec<_>, Vec<_>) = files.iter().partition(|f| has_ext(f, "properties"));
    for f in sets {
        let real = std::fs::canonicalize(f).map_err(|e| format!("{}: {e}", display_path(f)))?;
        im.root = real.parent().map(Path::to_path_buf).unwrap_or_default();
        let src = read(f)?;
        let insts = im
            .load(f, &real, &src)
            .map_err(|e| format!("{}: {e}", display_path(f)))?;
        im.insts.extend(insts);
        im.defines_set = true;
    }
    for f in props {
        let src = read(f)?;
        im.properties(f, &src);
    }
    Ok(im)
}

/// `file:line`, or just the file for line 0 (a JSON configuration's notes have no line).
fn place(file: &Path, line: usize) -> String {
    match line {
        0 => display_path(file),
        n => format!("{}:{n}", display_path(file)),
    }
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn read(p: &Path) -> Result<String, String> {
    let shown = display_path(p);
    let meta = std::fs::metadata(p).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{shown}: not a file"));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!("{shown}: larger than {MAX_BYTES} bytes"));
    }
    let bytes = std::fs::read(p).map_err(|e| format!("{shown}: {e}"))?;
    // checked again on what was read, in case the file grew
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{shown}: larger than {MAX_BYTES} bytes"));
    }
    let s = String::from_utf8(bytes).map_err(|_| format!("{shown}: not valid UTF-8"))?;
    Ok(s.strip_prefix('\u{feff}').unwrap_or(&s).to_string())
}

fn known(name: &str) -> bool {
    RULES.binary_search_by(|(n, _)| n.cmp(&name)).is_ok()
}

fn needs_types(name: &str) -> bool {
    RULES
        .binary_search_by(|(n, _)| n.cmp(&name))
        .is_ok_and(|i| RULES[i].1)
}

/// CodeNarc's WildcardPattern: a comma-separated list of names in which `*` and `?` match
/// any run of characters and any one character.
fn matches(patterns: &str, name: &str) -> bool {
    // the usual greedy match, going back only to the last `*`: linear in practice and never
    // exponential, whatever the pattern
    fn glob(p: &[u8], s: &[u8]) -> bool {
        let (mut i, mut j, mut star) = (0, 0, None);
        while j < s.len() {
            if i < p.len() && (p[i] == b'?' || p[i] == s[j]) {
                i += 1;
                j += 1;
            } else if i < p.len() && p[i] == b'*' {
                star = Some((i, j));
                i += 1;
            } else if let Some((si, sj)) = star {
                i = si + 1;
                j = sj + 1;
                star = Some((si, sj + 1));
            } else {
                return false;
            }
        }
        p[i..].iter().all(|&c| c == b'*')
    }
    patterns
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .any(|p| glob(p.as_bytes(), name.as_bytes()))
}

impl Import {
    /// The rules a ruleset file defines, by its format.
    fn load(&mut self, shown: &Path, real: &Path, src: &str) -> Result<Vec<Inst>, String> {
        if self.reading.len() >= MAX_REFS {
            return Err(format!(
                "rulesets reference each other more than {MAX_REFS} deep"
            ));
        }
        if self.reading.iter().any(|r| r == real) {
            return Err("a ruleset that references itself, directly or through others".into());
        }
        self.reading.push(real.to_path_buf());
        let name = shown.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let r = if has_ext(shown, "xml") {
            xml(src).and_then(|root| self.xml_ruleset(shown, &root))
        } else if has_ext(shown, "json") || name.starts_with(".groovylintrc") {
            if !has_ext(shown, "json") {
                Err("only .groovylintrc.json is read, not the .js or YAML forms".into())
            } else {
                self.groovylintrc(shown, src)
            }
        } else {
            self.dsl(shown, src)
        };
        self.reading.pop();
        r
    }

    fn note(&mut self, file: &Path, line: usize, what: impl std::fmt::Display) {
        self.notes.push(format!("{}: {what}", place(file, line)));
    }

    /// The rules of the ruleset at `path`, referenced from `from`: one CodeNarc bundles, or a
    /// file inside the input's directory. `None`, with a note, for one that can't be read.
    fn ruleset(&mut self, path: &str, from: &Path, line: usize) -> Option<Vec<Inst>> {
        let bare = path.strip_prefix("classpath:").unwrap_or(path);
        if let Some((p, members)) = RULESETS.iter().find(|(p, _)| *p == bare) {
            self.rulesets
                .push(format!("{p} from {}", place(from, line)));
            return Some(
                members
                    .iter()
                    .map(|&(n, on)| Inst {
                        name: n.into(),
                        on,
                        why: if on {
                            format!("in {p}")
                        } else {
                            format!("off in {p}")
                        },
                    })
                    .collect(),
            );
        }
        let rel = bare.strip_prefix("file:").unwrap_or(bare);
        let scheme = rel.split_once(':').is_some_and(|(s, _)| {
            s.len() > 1
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        });
        if scheme {
            self.note(
                from,
                line,
                format!("{path}: not read, only local files are"),
            );
            return None;
        }
        if Path::new(rel).is_absolute() {
            self.note(from, line, format!("{path}: not read, an absolute path"));
            return None;
        }
        // relative to the referencing file's directory, or to a directory above it (CodeNarc
        // reads `file:` paths from where the build runs), but only ever inside the input's
        // own directory
        let mut outside = false;
        let here = self.reading.last().cloned().unwrap_or_default();
        for d in here.ancestors().skip(1) {
            let Ok(real) = std::fs::canonicalize(d.join(rel)) else {
                continue;
            };
            if !real.is_file() {
                continue;
            }
            if !real.starts_with(&self.root) {
                outside = true;
                continue;
            }
            self.rulesets
                .push(format!("{rel} from {}", place(from, line)));
            let r = match read(&real) {
                // read's message names the file by its full path; the note names it as written
                Err(e) => Err(e.rsplit(": ").next().unwrap_or("").to_string()),
                Ok(src) => self.load(Path::new(rel), &real, &src),
            };
            return match r {
                Ok(v) => Some(v),
                Err(e) => {
                    self.note(from, line, format!("{path}: {e}"));
                    None
                }
            };
        }
        let why = if outside {
            "not read, it is outside the input file's directory"
        } else {
            "not found (a ruleset on the classpath, or a path relative to another directory)"
        };
        self.note(from, line, format!("{path}: {why}"));
        None
    }

    /// Applies a ruleset reference's configs and filters to its rules.
    fn filter(&mut self, mut rules: Vec<Inst>, f: Filter) -> Vec<Inst> {
        for (rule, prop, val, at) in f.configs {
            if !rules.iter().any(|r| r.name == rule) {
                self.notes.push(format!(
                    "{at}: {rule} isn't in the ruleset it configures (CodeNarc stops with an error)"
                ));
                continue;
            }
            if prop == "enabled" {
                match val.bool() {
                    Some(on) => {
                        for r in rules.iter_mut().filter(|r| r.name == rule) {
                            r.on = on;
                            r.why = format!("{} at {at}", if on { "enabled" } else { "disabled" });
                        }
                    }
                    None => self.notes.push(format!(
                        "{at}: {rule}.enabled = {}: not a literal true or false",
                        val.text()
                    )),
                }
            } else {
                self.props.insert((rule, prop, val.text()));
            }
        }
        let mut kept = Vec::new();
        for r in rules {
            let included = f.includes.is_empty() || f.includes.iter().any(|p| matches(p, &r.name));
            let excluded = f.excludes.iter().find(|(p, _)| matches(p, &r.name));
            match (included, excluded) {
                (true, None) => kept.push(r),
                (_, Some((_, at))) => kept.push(Inst {
                    on: false,
                    why: format!("excluded at {at}"),
                    ..r
                }),
                (false, None) => kept.push(Inst {
                    on: false,
                    why: "not included".into(),
                    ..r
                }),
            }
        }
        kept
    }

    fn xml_ruleset(&mut self, file: &Path, root: &El) -> Result<Vec<Inst>, String> {
        let at = |e: &El| format!("{}:{}", display_path(file), e.line);
        let mut out = Vec::new();
        for e in &root.kids {
            match e.name.as_str() {
                "description" => {}
                "ruleset-ref" => {
                    let path = e.need("path")?;
                    let mut f = Filter::default();
                    for k in &e.kids {
                        match k.name.as_str() {
                            "include" => f.includes.push(k.need("name")?.into()),
                            "exclude" => f.excludes.push((k.need("name")?.into(), at(k))),
                            "rule-config" => {
                                let rule = k.need("name")?;
                                for p in &k.kids {
                                    p.expect("property")?;
                                    f.configs.push((
                                        rule.into(),
                                        p.need("name")?.into(),
                                        Val::Lit(p.need("value")?.into()),
                                        at(p),
                                    ));
                                }
                            }
                            other => return Err(format!("line {}: unexpected <{other}>", k.line)),
                        }
                    }
                    if let Some(rules) = self.ruleset(path, file, e.line) {
                        out.extend(self.filter(rules, f));
                    }
                }
                "rule" => {
                    let class = e.need("class")?;
                    let name = class.rsplit('.').next().unwrap_or(class);
                    let name = name.strip_suffix("Rule").unwrap_or(name);
                    if !class.starts_with("org.codenarc.rule.") || !known(name) {
                        self.note(
                            file,
                            e.line,
                            format!("<rule class=\"{class}\">: a custom rule"),
                        );
                        continue;
                    }
                    let mut inst = Inst {
                        name: name.into(),
                        on: true,
                        why: format!("named at {}", at(e)),
                    };
                    for p in &e.kids {
                        p.expect("property")?;
                        let (prop, val) = (p.need("name")?, p.need("value")?);
                        self.rule_prop(&mut inst, prop, Val::Lit(val.into()), &at(p));
                    }
                    out.push(inst);
                }
                "rule-script" => {
                    self.note(
                        file,
                        e.line,
                        format!("<rule-script path=\"{}\">: a custom rule", e.need("path")?),
                    );
                }
                other => return Err(format!("line {}: unexpected <{other}>", e.line)),
            }
        }
        Ok(out)
    }

    /// A property set on a rule named on its own (not through a ruleset).
    fn rule_prop(&mut self, inst: &mut Inst, prop: &str, val: Val, at: &str) {
        if prop != "enabled" {
            self.props
                .insert((inst.name.clone(), prop.into(), val.text()));
            return;
        }
        match val.bool() {
            Some(on) => {
                inst.on = on;
                if !on {
                    inst.why = format!("disabled at {at}");
                }
            }
            None => self.notes.push(format!(
                "{at}: {}.enabled = {}: not a literal true or false",
                inst.name,
                val.text()
            )),
        }
    }

    /// The Groovy ruleset DSL: `ruleset { ... }` holding `ruleset('path') { ... }`, rule names
    /// and rule settings.
    fn dsl(&mut self, file: &Path, src: &str) -> Result<Vec<Inst>, String> {
        let tree = crate::parse(src);
        if let Some(e) = crate::first_error(&tree) {
            return Err(format!(
                "line {}: doesn't parse as Groovy",
                e.start_position().row + 1
            ));
        }
        let mut out = Vec::new();
        let mut found = false;
        let root = tree.root_node();
        for n in named(root) {
            match n.kind() {
                "package_declaration" | "import_declaration" => continue,
                _ => {}
            }
            match call(n, src) {
                Some(c) if c.name == "ruleset" && c.args.is_empty() && c.closure.is_some() => {
                    found = true;
                    let body = c.closure.unwrap_or(n);
                    self.dsl_top(file, src, body, &mut out);
                }
                _ => self.note(
                    file,
                    line(n),
                    format!("not interpreted: {}", snippet(n, src)),
                ),
            }
        }
        if !found {
            return Err("no `ruleset { }` block: not a CodeNarc ruleset".into());
        }
        Ok(out)
    }

    fn dsl_top(&mut self, file: &Path, src: &str, body: Node, out: &mut Vec<Inst>) {
        let at = |n: Node| format!("{}:{}", display_path(file), line(n));
        for n in named(body) {
            let Some(c) = call(n, src) else {
                self.note(
                    file,
                    line(n),
                    format!("not interpreted: {}", snippet(n, src)),
                );
                continue;
            };
            match c.name.as_str() {
                "description" => {}
                "ruleset" => {
                    let path = match c.args.as_slice() {
                        [a] => literal(*a, src),
                        _ => None,
                    };
                    let Some(path) = path else {
                        self.note(file, line(n), format!("not interpreted: {}", snippet(n, src)));
                        continue;
                    };
                    let mut f = Filter::default();
                    let mut extra = Vec::new();
                    if let Some(cl) = c.closure {
                        self.dsl_ruleset_body(file, src, cl, &mut f, &mut extra);
                    }
                    if let Some(rules) = self.ruleset(&path, file, line(n)) {
                        out.extend(self.filter(rules, f));
                    }
                    out.extend(extra);
                }
                "rule" => self.note(
                    file,
                    line(n),
                    format!("{}: a custom rule", snippet(n, src)),
                ),
                name if known(name) => {
                    let mut inst = Inst {
                        name: name.into(),
                        on: true,
                        why: format!("named at {}", at(n)),
                    };
                    for (prop, val, pn) in self.settings(file, src, &c) {
                        self.rule_prop(&mut inst, &prop, val, &at(pn));
                    }
                    out.push(inst);
                }
                name => self.note(
                    file,
                    line(n),
                    format!("{name}: not a CodeNarc {CODENARC_VERSION} rule (CodeNarc stops with an error)"),
                ),
            }
        }
    }

    /// The body of `ruleset('path') { ... }`: excludes, includes, rule settings, and bare rule
    /// names, which Groovy resolves to the enclosing `ruleset { }`, so they add the rule.
    fn dsl_ruleset_body(
        &mut self,
        file: &Path,
        src: &str,
        body: Node,
        f: &mut Filter,
        extra: &mut Vec<Inst>,
    ) {
        let at = |n: Node| format!("{}:{}", display_path(file), line(n));
        for n in named(body) {
            let c = call(n, src);
            match c {
                Some(c)
                    if matches!(c.name.as_str(), "exclude" | "include") && c.closure.is_none() =>
                {
                    let pats: Option<Vec<String>> =
                        c.args.iter().map(|a| literal(*a, src)).collect();
                    match pats {
                        Some(pats) if pats.len() == 1 => {
                            if c.name == "exclude" {
                                f.excludes.push((pats[0].clone(), at(n)));
                            } else {
                                f.includes.push(pats[0].clone());
                            }
                        }
                        _ => self.note(
                            file,
                            line(n),
                            format!("not interpreted: {}", snippet(n, src)),
                        ),
                    }
                }
                Some(c) if c.bare && known(&c.name) => extra.push(Inst {
                    name: c.name.clone(),
                    on: true,
                    why: format!("named at {}", at(n)),
                }),
                Some(c) if !c.bare && known(&c.name) => {
                    for (prop, val, pn) in self.settings(file, src, &c) {
                        f.configs.push((c.name.clone(), prop, val, at(pn)));
                    }
                }
                _ => self.note(
                    file,
                    line(n),
                    format!("not interpreted: {}", snippet(n, src)),
                ),
            }
        }
    }

    /// The properties a rule call sets: `Rule(a: 1)` or `Rule { a = 1 }`.
    fn settings<'t>(
        &mut self,
        file: &Path,
        src: &str,
        c: &Call<'t>,
    ) -> Vec<(String, Val, Node<'t>)> {
        let mut out = Vec::new();
        for &a in &c.args {
            match a.kind() {
                "named_argument" => {
                    let key = a.child_by_field_name("key").and_then(|k| name_of(k, src));
                    let val = a.child_by_field_name("value");
                    match (key, val) {
                        (Some(k), Some(v)) => out.push((k, value(v, src), a)),
                        _ => self.note(
                            file,
                            line(a),
                            format!("not interpreted: {}", snippet(a, src)),
                        ),
                    }
                }
                _ => self.note(
                    file,
                    line(a),
                    format!("not interpreted: {}", snippet(a, src)),
                ),
            }
        }
        if let Some(cl) = c.closure {
            for n in named(cl) {
                let assign = (n.kind() == "expression_statement")
                    .then(|| n.named_child(0))
                    .flatten()
                    .filter(|e| e.kind() == "assignment_expression")
                    .filter(|e| {
                        e.child_by_field_name("operator")
                            .is_some_and(|o| o.kind() == "=")
                    });
                let left = assign
                    .and_then(|e| e.child_by_field_name("left"))
                    .filter(|l| l.kind() == "identifier");
                match (left, assign.and_then(|e| e.child_by_field_name("right"))) {
                    (Some(l), Some(r)) => out.push((text(l, src).into(), value(r, src), n)),
                    _ => self.note(
                        file,
                        line(n),
                        format!("not interpreted: {}", snippet(n, src)),
                    ),
                }
            }
        }
        out
    }

    /// `.groovylintrc.json`, as npm-groovy-lint reads it: `extends` presets, then `rules`,
    /// where a rule is off as "off" or with `enabled: false`, `disabled: true` or
    /// `severity: "off"`. With `codenarcRulesets`, those files replace `rules`.
    fn groovylintrc(&mut self, file: &Path, src: &str) -> Result<Vec<Inst>, String> {
        let v = crate::json::parse(&strip_json_comments(src))?;
        let crate::json::Value::Obj(top) = &v else {
            return Err("not a .groovylintrc.json (not a JSON object)".into());
        };
        let shown = display_path(file);
        if let Some(sets) = v.get("codenarcRulesets").str() {
            if !v.get("rules").is_null() {
                self.note(
                    file,
                    0,
                    "rules: ignored, as npm-groovy-lint does when codenarcRulesets is set",
                );
            }
            let mut out = Vec::new();
            for s in sets.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if let Some(r) = self.ruleset(s, file, 0) {
                    out.extend(r);
                }
            }
            return Ok(out);
        }
        for (k, _) in top {
            if !matches!(k.as_str(), "extends" | "rules") {
                self.note(file, 0, format!("{k}: not about rules, ignored"));
            }
        }
        // rule -> (on, why), in the order npm-groovy-lint merges them
        let mut rules: BTreeMap<String, (bool, String)> = BTreeMap::new();
        let ext: Vec<&str> = match v.get("extends") {
            crate::json::Value::Str(s) => vec![s.as_str()],
            e => e.arr().iter().filter_map(|x| x.str()).collect(),
        };
        for e in ext {
            if !self.preset(e, &mut rules, 0) {
                self.note(
                    file,
                    0,
                    format!("extends {e}: not an npm-groovy-lint {NPM_GROOVY_LINT_VERSION} preset"),
                );
            }
        }
        if let crate::json::Value::Obj(rs) = v.get("rules") {
            for (k, val) in rs {
                // npm-groovy-lint drops the category: `basic.DeadCode` is DeadCode
                let name = k.rsplit('.').next().unwrap_or(k);
                if !known(name) {
                    self.note(
                        file,
                        0,
                        format!("rules.{k}: not a CodeNarc {CODENARC_VERSION} rule"),
                    );
                    continue;
                }
                use crate::json::Value::{Bool, Obj, Str};
                let off = match val {
                    Str(s) => s == "off",
                    Obj(_) => {
                        val.get("severity").str() == Some("off")
                            || *val.get("disabled") == Bool(true)
                            || *val.get("enabled") == Bool(false)
                    }
                    _ => false,
                };
                if let Obj(m) = val {
                    for (p, pv) in m {
                        if !matches!(p.as_str(), "enabled" | "disabled" | "severity") {
                            self.props.insert((name.into(), p.clone(), json_text(pv)));
                        }
                    }
                }
                let why = if off {
                    format!("disabled in {shown}")
                } else {
                    format!("named in {shown}")
                };
                rules.insert(name.into(), (!off, why));
            }
        }
        Ok(rules
            .into_iter()
            .map(|(name, (on, why))| Inst { name, on, why })
            .collect())
    }

    /// Merges npm-groovy-lint's preset `name` (and the ones it extends) into `rules`.
    fn preset(
        &mut self,
        name: &str,
        rules: &mut BTreeMap<String, (bool, String)>,
        depth: usize,
    ) -> bool {
        let Some((_, ext, rs)) = PRESETS.iter().find(|(n, _, _)| *n == name) else {
            return false;
        };
        if depth < MAX_REFS {
            for e in *ext {
                self.preset(e, rules, depth + 1);
            }
        }
        for &(r, on) in *rs {
            let why = if on {
                format!("in preset {name}")
            } else {
                format!("off in preset {name}")
            };
            rules.insert(r.into(), (on, why));
        }
        true
    }

    /// `codenarc.properties`: `Rule.property=value` lines that configure the rules the
    /// rulesets define.
    fn properties(&mut self, file: &Path, src: &str) {
        let mut lines = src.lines().enumerate();
        while let Some((i, l)) = lines.next() {
            let mut l = l.trim_start().to_string();
            if l.is_empty() || l.starts_with('#') || l.starts_with('!') {
                continue;
            }
            // a line ending in an odd number of backslashes continues on the next
            while l.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1 {
                l.pop();
                match lines.next() {
                    Some((_, next)) => l.push_str(next.trim_start()),
                    None => break,
                }
            }
            let cut = l.find(['=', ':']).unwrap_or(l.len());
            let (key, val) = (l[..cut].trim(), l.get(cut + 1..).unwrap_or("").trim());
            let Some((rule, prop)) = key.split_once('.') else {
                continue;
            };
            if !rule
                .chars()
                .chain(prop.chars())
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                continue;
            }
            let at = format!("{}:{}", display_path(file), i + 1);
            if !known(rule) {
                self.notes.push(format!(
                    "{at}: {rule}: not a CodeNarc {CODENARC_VERSION} rule"
                ));
                continue;
            }
            if prop != "enabled" {
                self.props.insert((rule.into(), prop.into(), val.into()));
                continue;
            }
            let Some(on) = Val::Lit(val.into()).bool() else {
                self.notes
                    .push(format!("{at}: {rule}.enabled = {val}: not true or false"));
                continue;
            };
            let why = format!("{} at {at}", if on { "enabled" } else { "disabled" });
            let mut found = false;
            for r in self.insts.iter_mut().filter(|r| r.name == rule) {
                (r.on, r.why, found) = (on, why.clone(), true);
            }
            if !found && self.defines_set {
                self.notes.push(format!(
                    "{at}: {rule} isn't in the rulesets, so this does nothing (as in CodeNarc)"
                ));
            } else if !found {
                self.insts.push(Inst {
                    name: rule.into(),
                    on,
                    why,
                });
            }
        }
    }

    /// Each CodeNarc rule the source mentions: on when any instance of it is.
    fn states(&self) -> BTreeMap<&str, bool> {
        let mut m: BTreeMap<&str, bool> = BTreeMap::new();
        for i in &self.insts {
            *m.entry(&i.name).or_default() |= i.on;
        }
        m
    }

    /// The `.groovyfmt.toml` text: suggested formatter settings, the `[lint]` table, and
    /// the report of what isn't carried over, as comments.
    pub fn toml(&self) -> String {
        let states = self.states();
        let ours = |n: &str| crate::lint::RULES.iter().any(|r| r.name == n);
        let on: Vec<&str> = states
            .iter()
            .filter(|(_, on)| **on)
            .map(|(n, _)| *n)
            .collect();
        let fmt: Vec<(&str, &str)> = FORMATTER
            .iter()
            .filter(|(n, _)| states.get(n) == Some(&true))
            .copied()
            .collect();
        let typed: Vec<&str> = on.iter().copied().filter(|n| needs_types(n)).collect();
        let missing: Vec<&str> = on
            .iter()
            .copied()
            .filter(|n| !ours(n) && !needs_types(n) && !fmt.iter().any(|(f, _)| f == n))
            .collect();

        let mut o = format!(
            "# groovyfmt --import-codenarc {}\n# (CodeNarc {CODENARC_VERSION} rulesets, npm-groovy-lint {NPM_GROOVY_LINT_VERSION} presets)\n",
            self.inputs.join(" ")
        );
        // formatter settings go before [lint]; commented out, since they reformat files
        let indent = self
            .props
            .iter()
            .find(|(r, p, v)| {
                r == "Indentation"
                    && p == "spacesPerIndentLevel"
                    && v.parse::<u8>().is_ok_and(|n| (1..=16).contains(&n))
            })
            .filter(|_| states.get("Indentation") == Some(&true));
        let spacing = fmt.iter().any(|(_, how)| *how == SPACING);
        if indent.is_some() || spacing {
            o += "\n# The formatter covers some formatting rules the source enables with these settings:\n";
            if let Some((_, _, n)) = indent {
                o += &format!("# indent = {n}\n");
            }
            if spacing {
                o += &format!("# {SPACING}\n");
            }
        }
        o += "\n[lint]\n";
        for r in crate::lint::RULES {
            if let Some(&on) = states.get(r.name) {
                o += &format!("{} = {on}\n", r.name);
            } else if self.defines_set && known(r.name) {
                o += &format!("{} = false\n", r.name);
            }
        }
        if self.defines_set {
            o += "# groovyfmt's Jenkins, Gradle and Spock rules and UnusedSuppression keep their defaults.\n";
        }

        let mut list = |title: &str, items: Vec<String>| {
            if items.is_empty() {
                return;
            }
            o += &format!("\n# {title} ({}):\n", items.len());
            o += &wrap(&items);
        };
        list(
            "Enabled in the source, no groovyfmt equivalent yet",
            missing.iter().map(|s| s.to_string()).collect(),
        );
        list(
            "Enabled in the source, needs type information (CodeNarc compiles the code for it), out of scope",
            typed.iter().map(|s| s.to_string()).collect(),
        );
        list(
            "Formatting rules groovyfmt's formatter covers",
            fmt.iter().map(|(n, how)| format!("{n}: {how}")).collect(),
        );
        // properties matter only on rules carried over
        let kept =
            |r: &str| states.get(r) == Some(&true) && (ours(r) || fmt.iter().any(|(f, _)| *f == r));
        list(
            "Rule properties ignored",
            self.props
                .iter()
                .filter(|(r, _, _)| kept(r))
                .map(|(r, p, v)| format!("{r}.{p} = {}", shorten(v)))
                .collect(),
        );
        list("Could not interpret", self.notes.clone());
        o
    }

    /// For `-v`: the rulesets read, each rule instance, and the properties set.
    pub fn details(&self) -> String {
        let mut o = String::new();
        for r in &self.rulesets {
            o += &format!("ruleset {r}\n");
        }
        for i in &self.insts {
            o += &format!(
                "rule {} {}: {}\n",
                i.name,
                if i.on { "on" } else { "off" },
                i.why
            );
        }
        for (r, p, v) in &self.props {
            o += &format!("property {r}.{p} = {}\n", shorten(v));
        }
        o
    }
}

/// Rule names filled to about 90 columns, other items (with spaces) one per line; `#   ` first.
fn wrap(items: &[String]) -> String {
    if items.iter().any(|i| i.contains(' ')) {
        return items.iter().map(|i| format!("#   {i}\n")).collect();
    }
    let mut o = String::new();
    let mut line = String::new();
    for (k, i) in items.iter().enumerate() {
        let piece = if k + 1 < items.len() {
            format!("{i},")
        } else {
            i.clone()
        };
        if !line.is_empty() && line.len() + 1 + piece.len() > 86 {
            o += &format!("#   {line}\n");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line += &piece;
    }
    o + &format!("#   {line}\n")
}

/// A value on one line, at most 60 characters.
fn shorten(v: &str) -> String {
    let one: String = v.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 {
        format!("{}...", one.chars().take(57).collect::<String>())
    } else {
        one
    }
}

fn json_text(v: &crate::json::Value) -> String {
    use crate::json::Value::*;
    match v {
        Null => "null".into(),
        Bool(b) => b.to_string(),
        Num(n) => n.clone(),
        Str(s) => s.clone(),
        Arr(a) => format!(
            "[{}]",
            a.iter().map(json_text).collect::<Vec<_>>().join(", ")
        ),
        Obj(_) => "{...}".into(),
    }
}

/// `//` and `/* */` comments outside strings removed, as npm-groovy-lint does.
fn strip_json_comments(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    let mut in_str = false;
    while let Some(c) = it.next() {
        if in_str {
            o.push(c);
            match c {
                '\\' => o.extend(it.next()),
                '"' => in_str = false,
                _ => {}
            }
        } else if c == '"' {
            in_str = true;
            o.push(c);
        } else if c == '/' && it.peek() == Some(&'/') {
            while it.next_if(|&c| c != '\n').is_some() {}
        } else if c == '/' && it.peek() == Some(&'*') {
            it.next();
            let mut prev = ' ';
            for c in it.by_ref() {
                if prev == '*' && c == '/' {
                    break;
                }
                // keep line breaks, so error positions stay meaningful
                if c == '\n' {
                    o.push('\n');
                }
                prev = c;
            }
        } else {
            o.push(c);
        }
    }
    o
}

// The DSL reader's view of statements.

fn line(n: Node) -> usize {
    n.start_position().row + 1
}

/// The statements (or arguments) of a node, without comments.
fn named(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c)
        .filter(|k| !k.kind().contains("comment"))
        .collect()
}

/// The first line of a node's text, for a note.
fn snippet(n: Node, src: &str) -> String {
    shorten(text(n, src).lines().next().unwrap_or(""))
}

/// A call as the DSL writes one: `name`, `name(args)`, `name args`, `name { }`,
/// `name(args) { }`, with the name an identifier or a plain string.
struct Call<'t> {
    name: String,
    args: Vec<Node<'t>>,
    closure: Option<Node<'t>>,
    /// Just a name, no arguments or closure.
    bare: bool,
}

fn call<'t>(n: Node<'t>, src: &str) -> Option<Call<'t>> {
    let n = if n.kind() == "expression_statement" {
        n.named_child(0)?
    } else {
        n
    };
    match n.kind() {
        "identifier" => Some(Call {
            name: text(n, src).into(),
            args: Vec::new(),
            closure: None,
            bare: true,
        }),
        "method_invocation" => {
            let name = name_of(n.child_by_field_name("function")?, src)?;
            // `a.b(...)` has an object; only plain calls are DSL statements
            if n.child_by_field_name("object").is_some() {
                return None;
            }
            let args = n
                .child_by_field_name("arguments")
                .map(named)
                .unwrap_or_default();
            Some(Call {
                name,
                args,
                closure: n.child_by_field_name("closure"),
                bare: false,
            })
        }
        "command_chain" => {
            let name = name_of(n.child_by_field_name("receiver")?, src)?;
            let mut c = n.walk();
            let mut args: Vec<Node> = n.children_by_field_name("argument", &mut c).collect();
            // a command chain's arguments are its only other children
            if named(n).len() != args.len() + 1 {
                return None;
            }
            let closure = args.last().copied().filter(|a| a.kind() == "closure");
            if closure.is_some() {
                args.pop();
            }
            Some(Call {
                name,
                args,
                closure,
                bare: false,
            })
        }
        _ => None,
    }
}

/// An identifier, or a string without interpolation, as a name.
fn name_of(n: Node, src: &str) -> Option<String> {
    match n.kind() {
        "identifier" => Some(text(n, src).into()),
        _ => literal(n, src),
    }
}

/// The text of a string literal without interpolation.
fn literal(n: Node, src: &str) -> Option<String> {
    if n.kind() != "string_literal" || {
        let mut c = n.walk();
        n.named_children(&mut c)
            .any(|k| k.kind().starts_with("gstring_"))
    } {
        return None;
    }
    let t = text(n, src);
    let q = ["'''", "\"\"\"", "'", "\"", "/"]
        .iter()
        .find(|q| t.starts_with(**q) && t.ends_with(**q) && t.len() >= 2 * q.len())?;
    Some(t[q.len()..t.len() - q.len()].to_string())
}

fn value(n: Node, src: &str) -> Val {
    match n.kind() {
        "boolean_literal" | "number_literal" => Val::Lit(text(n, src).into()),
        _ => literal(n, src).map_or_else(|| Val::Code(text(n, src).into()), Val::Lit),
    }
}

// A reader for CodeNarc's ruleset XML and nothing else: elements, attributes, comments,
// CDATA and processing instructions; text is skipped. No DTD, so no entities beyond XML's
// five and character references.

struct El {
    name: String,
    attrs: Vec<(String, String)>,
    kids: Vec<El>,
    line: usize,
}

impl El {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn need(&self, name: &str) -> Result<&str, String> {
        self.attr(name)
            .ok_or_else(|| format!("line {}: <{}> without {name}", self.line, self.name))
    }
    fn expect(&self, name: &str) -> Result<(), String> {
        if self.name == name {
            Ok(())
        } else {
            Err(format!("line {}: unexpected <{}>", self.line, self.name))
        }
    }
}

struct Xml<'a> {
    s: &'a str,
    i: usize,
}

fn xml(s: &str) -> Result<El, String> {
    let mut x = Xml { s, i: 0 };
    x.misc()?;
    let root = x.element(0)?;
    x.misc()?;
    if x.i != s.len() {
        return Err(format!("line {}: text after the root element", x.line()));
    }
    Ok(root)
}

impl Xml<'_> {
    fn line(&self) -> usize {
        self.s[..self.i].matches('\n').count() + 1
    }
    fn rest(&self) -> &str {
        &self.s[self.i..]
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("line {}: {what}", self.line()))
    }
    fn ws(&mut self) {
        let n = self.rest().len() - self.rest().trim_start().len();
        self.i += n;
    }
    /// Skips past `end`.
    fn past(&mut self, end: &str) -> Result<(), String> {
        match self.rest().find(end) {
            Some(k) => {
                self.i += k + end.len();
                Ok(())
            }
            None => self.err(&format!("no closing {end}")),
        }
    }
    /// Whitespace, comments and processing instructions; a DTD is refused.
    fn misc(&mut self) -> Result<(), String> {
        loop {
            self.ws();
            if self.rest().starts_with("<!--") {
                self.past("-->")?;
            } else if self.rest().starts_with("<?") {
                self.past("?>")?;
            } else if self.rest().starts_with("<!") {
                return self.err("DTDs and declarations are not supported");
            } else {
                return Ok(());
            }
        }
    }
    fn name(&mut self) -> Result<String, String> {
        let n = self
            .rest()
            .find(|c: char| !(c.is_ascii_alphanumeric() || "-_.:".contains(c)))
            .unwrap_or(self.rest().len());
        if n == 0 {
            return self.err("expected a name");
        }
        let name = self.rest()[..n].to_string();
        self.i += n;
        Ok(name)
    }
    fn element(&mut self, depth: usize) -> Result<El, String> {
        if depth >= MAX_XML_DEPTH {
            return self.err(&format!("nested more than {MAX_XML_DEPTH} deep"));
        }
        let line = self.line();
        if !self.rest().starts_with('<') {
            return self.err("expected an element");
        }
        self.i += 1;
        let name = self.name()?;
        if name.contains(':') {
            return self.err(&format!("<{name}>: prefixed elements are not supported"));
        }
        let mut el = El {
            name,
            attrs: Vec::new(),
            kids: Vec::new(),
            line,
        };
        loop {
            self.ws();
            // the root is checked before anything inside it is read
            let end = self.rest().starts_with("/>") || self.rest().starts_with('>');
            if end && depth == 0 && (el.name != "ruleset" || el.attr("xmlns") != Some(NAMESPACE)) {
                return Err(format!(
                    "not a CodeNarc ruleset (a <ruleset> in the {NAMESPACE} namespace)"
                ));
            }
            if self.rest().starts_with("/>") {
                self.i += 2;
                return Ok(el);
            }
            if self.rest().starts_with('>') {
                self.i += 1;
                break;
            }
            let k = self.name()?;
            self.ws();
            if !self.rest().starts_with('=') {
                return self.err("expected `=`");
            }
            self.i += 1;
            self.ws();
            let q = match self.rest().chars().next() {
                Some(q @ ('"' | '\'')) => q,
                _ => return self.err("expected a quoted value"),
            };
            self.i += 1;
            let Some(end) = self.rest().find(q) else {
                return self.err("unterminated value");
            };
            let raw = &self.rest()[..end];
            if raw.contains('<') {
                return self.err("`<` in an attribute value");
            }
            let v = unescape(raw).map_err(|e| format!("line {}: {e}", self.line()))?;
            self.i += end + 1;
            if el.attrs.iter().any(|(a, _)| *a == k) {
                return self.err(&format!("attribute {k} twice"));
            }
            el.attrs.push((k, v));
        }
        loop {
            let text = self.rest().find('<').unwrap_or(self.rest().len());
            unescape(&self.rest()[..text]).map_err(|e| format!("line {}: {e}", self.line()))?;
            self.i += text;
            let r = self.rest();
            if r.is_empty() {
                return self.err(&format!("<{}> isn't closed", el.name));
            } else if r.starts_with("</") {
                self.i += 2;
                let close = self.name()?;
                self.ws();
                if close != el.name || !self.rest().starts_with('>') {
                    return self.err(&format!("expected </{}>", el.name));
                }
                self.i += 1;
                return Ok(el);
            } else if r.starts_with("<!--") {
                self.past("-->")?;
            } else if r.starts_with("<![CDATA[") {
                self.past("]]>")?;
            } else if r.starts_with("<?") {
                self.past("?>")?;
            } else if r.starts_with("<!") {
                return self.err("declarations are not supported");
            } else {
                let kid = self.element(depth + 1)?;
                el.kids.push(kid);
            }
        }
    }
}

/// XML's five entities and character references; anything else would need a DTD.
fn unescape(s: &str) -> Result<String, String> {
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        o += &rest[..k];
        rest = &rest[k + 1..];
        let end = rest.find(';').ok_or("`&` without `;`")?;
        let ent = &rest[..end];
        let c = match ent {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let n = if let Some(h) = ent.strip_prefix("#x") {
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = ent.strip_prefix('#') {
                    d.parse().ok()
                } else {
                    None
                };
                // XML allows no other control characters
                n.and_then(char::from_u32)
                    .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
                    .ok_or_else(|| format!("entity &{ent}; isn't supported (no DTDs)"))?
            }
        };
        o.push(c);
        rest = &rest[end + 1..];
    }
    Ok(o + rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let p = std::env::temp_dir()
                .join(format!("groovyfmt-codenarc-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn write(&self, rel: &str, text: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            p
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn states(files: &[PathBuf]) -> (BTreeMap<String, bool>, Import) {
        let im = import(files).unwrap();
        let s = im
            .states()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        (s, im)
    }

    const NS: &str = "<ruleset xmlns=\"http://codenarc.org/ruleset/1.0\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">";

    #[test]
    fn data_tables_are_sorted_and_consistent() {
        assert!(RULES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(RULES.len() > 400);
        for (_, members) in RULESETS {
            assert!(members.iter().all(|(n, _)| known(n)), "unknown member");
        }
        for (_, ext, rs) in PRESETS {
            assert!(rs.iter().all(|(n, _)| known(n)));
            assert!(ext.iter().all(|e| PRESETS.iter().any(|(p, _, _)| p == e)));
        }
        for (n, _) in FORMATTER {
            assert!(known(n), "{n}");
        }
        // every general groovyfmt rule named after CodeNarc is a CodeNarc rule
        for r in crate::lint::RULES {
            let ours = ["Jenkins", "Gradle", "Spock"]
                .iter()
                .any(|p| r.name.starts_with(p))
                || r.name == "UnusedSuppression";
            assert_eq!(known(r.name), !ours, "{}", r.name);
        }
    }

    #[test]
    fn xml_ruleset_refs_excludes_and_configs() {
        let d = Dir::new("xml");
        let f = d.write(
            "codenarc.xml",
            &format!(
                "<?xml version=\"1.0\"?>\n<!-- a comment -->\n{NS}\n<description><![CDATA[x < y]]> &amp; more</description>\n\
                 <ruleset-ref path='rulesets/basic.xml'>\n<exclude name='Empty*Block'/>\n\
                 <rule-config name='DeadCode'><property name='priority' value='1'/></rule-config>\n\
                 <rule-config name='ComparisonWithSelf'><property name='enabled' value='false'/></rule-config>\n\
                 </ruleset-ref>\n<ruleset-ref path='rulesets/imports.xml'><include name='UnusedImport,DuplicateImport'/></ruleset-ref>\n\
                 <rule class='org.codenarc.rule.unnecessary.UnnecessaryGStringRule'><property name='enabled' value='true'/></rule>\n\
                 <rule class='com.example.MyRule'/>\n</ruleset>\n"
            ),
        );
        let (s, im) = states(&[f]);
        assert!(s["DeadCode"]);
        assert!(!s["EmptyCatchBlock"]);
        assert!(!s["EmptyFinallyBlock"]);
        assert!(
            s["EmptyIfStatement"],
            "EmptyIfStatement doesn't match Empty*Block"
        );
        assert!(!s["ComparisonWithSelf"]);
        assert!(s["UnusedImport"]);
        assert!(!s["UnnecessaryGroovyImport"]);
        assert!(s["UnnecessaryGString"]);
        let t = im.toml();
        assert!(t.contains("\nDeadCode = true\n"), "{t}");
        assert!(t.contains("\nUnnecessaryGroovyImport = false\n"), "{t}");
        assert!(t.contains("\nEmptyCatchBlock = false\n"), "{t}");
        assert!(t.contains("DeadCode.priority = 1"), "{t}");
        assert!(t.contains("com.example.MyRule\">: a custom rule"), "{t}");
        // basic's rules groovyfmt lacks
        assert!(t.contains("ExplicitGarbageCollection"), "{t}");
        assert!(!t.contains("JenkinsMissingSection"), "{t}");
        assert!(
            im.details()
                .contains("rule EmptyCatchBlock off: excluded at ")
        );
    }

    #[test]
    fn xml_rejects_what_it_does_not_understand() {
        let d = Dir::new("xmlbad");
        let cases = [
            ("<!DOCTYPE r [<!ENTITY a \"aaaa\">]>\n<ruleset/>", "DTDs"),
            (
                &format!("{NS}<ruleset-ref path='&a;'/></ruleset>"),
                "entity &a;",
            ),
            (
                &format!("{NS}<ruleset-ref path='&#0;'/></ruleset>"),
                "entity &#0;",
            ),
            (
                "<ruleset><ruleset-ref path='rulesets/basic.xml'/></ruleset>",
                "namespace",
            ),
            (&format!("{NS}<rules/></ruleset>"), "unexpected <rules>"),
            (&format!("{NS}<ruleset-ref/></ruleset>"), "without path"),
            (&format!("{NS}<ruleset-ref path='a'>"), "isn't closed"),
            (&format!("{NS}<a:b/></ruleset>"), "prefixed"),
            (&format!("{NS}</rulesetx>"), "expected </ruleset>"),
            (
                "<x xmlns=\"http://codenarc.org/ruleset/1.0\"/>",
                "not a CodeNarc ruleset",
            ),
            (&format!("{NS}</ruleset><ruleset/>"), "after the root"),
        ];
        for (src, want) in cases {
            let f = d.write("c.xml", src);
            let e = import(&[f]).err().unwrap_or_default();
            assert!(e.contains(want), "{src}: {e}");
        }
        // deep nesting stops at the cap, without recursion trouble
        let deep = format!(
            "{NS}{}{}</ruleset>",
            "<description>".repeat(10_000),
            "</description>".repeat(10_000)
        );
        let f = d.write("c.xml", &deep);
        assert!(import(&[f]).err().unwrap().contains("nested more than"));
        // too large
        let f = d.write(
            "c.xml",
            &format!("{NS}<!--{}--></ruleset>", "x".repeat(MAX_BYTES as usize)),
        );
        assert!(import(&[f]).err().unwrap().contains("larger than"));
    }

    #[test]
    fn refs_stay_inside_the_input_directory() {
        let d = Dir::new("refs");
        d.write(
            "secret.xml",
            &format!("{NS}<ruleset-ref path='rulesets/basic.xml'/></ruleset>"),
        );
        d.write(
            "cfg/codenarc/base.groovy",
            "ruleset {\n    ruleset('rulesets/imports.xml')\n}\n",
        );
        let f = d.write(
            "cfg/codenarc/main.groovy",
            "ruleset {\n    ruleset('../../secret.xml')\n    ruleset('file:cfg/codenarc/base.groovy') {\n        exclude 'UnusedImport'\n    }\n    ruleset('http://example.com/r.xml')\n    ruleset('/etc/r.xml')\n    ruleset('main.groovy')\n    ruleset('nope.xml')\n}\n",
        );
        let (s, im) = states(&[f]);
        let t = im.toml();
        assert!(
            !s.contains_key("DeadCode"),
            "read a file outside the directory: {t}"
        );
        assert!(
            t.contains("../../secret.xml: not read, it is outside"),
            "{t}"
        );
        assert!(!s["UnusedImport"]);
        assert!(s["DuplicateImport"]);
        assert!(
            t.contains("http://example.com/r.xml: not read, only local files are"),
            "{t}"
        );
        assert!(t.contains("/etc/r.xml: not read, an absolute path"), "{t}");
        assert!(
            t.contains("main.groovy: a ruleset that references itself"),
            "{t}"
        );
        assert!(t.contains("nope.xml: not found"), "{t}");
        // a symlink out of the directory is outside too
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.0.join("secret.xml"), d.0.join("cfg/codenarc/link.xml"))
                .unwrap();
            let f = d.write(
                "cfg/codenarc/l.groovy",
                "ruleset {\n    ruleset('link.xml')\n}\n",
            );
            let (s, im) = states(&[f]);
            assert!(!s.contains_key("DeadCode"));
            assert!(im.toml().contains("link.xml: not read, it is outside"));
        }
    }

    #[test]
    fn dsl_declarative_forms() {
        let d = Dir::new("dsl");
        let f = d.write(
            "rules.groovy",
            "import foo.Bar\n// a comment\nruleset {\n    description 'mine'\n    ruleset('rulesets/basic.xml') {\n        exclude 'EmptyCatchBlock'\n        'DeadCode' {\n            priority = 1\n            enabled = false\n        }\n        ComparisonWithSelf(enabled: false)\n        EmptyIfStatement enabled: false\n        UnnecessaryGString\n        EmptyClass { doNotApplyToClassNames = 'A, B' }\n        println 'hi'\n    }\n    UnusedImport\n    LineLength { length = 120 }\n    Indentation(spacesPerIndentLevel: 2)\n    TrailingWhitespace\n    rule(com.example.MyRule)\n    NoSuchRule\n    DuplicateImport { enabled = flag }\n}\nprintln 'x'\n",
        );
        let (s, im) = states(&[f]);
        let t = im.toml();
        assert!(!s["EmptyCatchBlock"]);
        assert!(!s["DeadCode"]);
        assert!(!s["ComparisonWithSelf"]);
        assert!(!s["EmptyIfStatement"]);
        assert!(
            s["UnnecessaryGString"],
            "a bare name in a ruleset adds the rule"
        );
        assert!(s["UnusedImport"]);
        assert!(s["DuplicateImport"]);
        assert!(t.contains("println 'hi'"), "{t}");
        assert!(t.contains("println 'x'"), "{t}");
        assert!(t.contains("rule(com.example.MyRule): a custom rule"), "{t}");
        assert!(t.contains("NoSuchRule: not a CodeNarc"), "{t}");
        assert!(
            t.contains("DuplicateImport.enabled = flag: not a literal"),
            "{t}"
        );
        assert!(t.contains("# indent = 2\n"), "{t}");
        assert!(
            t.contains("TrailingWhitespace: trailing whitespace is always removed"),
            "{t}"
        );
        assert!(t.contains("LineLength"), "{t}");
        // a property of a rule that isn't carried over isn't listed
        assert!(!t.contains("doNotApplyToClassNames"), "{t}");
        // the output is a valid config
        let cfg = crate::config::Config::parse(&t).unwrap();
        assert!(!cfg.rule_enabled("DeadCode"));
        assert!(cfg.rule_enabled("UnnecessaryGString"));
    }

    #[test]
    fn dsl_that_is_not_a_ruleset_is_an_error() {
        let d = Dir::new("dslbad");
        let f = d.write("a.groovy", "println 'x'\n");
        assert!(
            import(&[f])
                .err()
                .unwrap()
                .contains("no `ruleset { }` block")
        );
        let f = d.write("b.groovy", "ruleset {\n    ruleset('x'\n}\n");
        assert!(import(&[f]).err().unwrap().contains("doesn't parse"));
    }

    #[test]
    fn groovylintrc_extends_and_rules() {
        let d = Dir::new("ngl");
        let f = d.write(
            ".groovylintrc.json",
            "{\n  // comment\n  \"extends\": \"recommended\",\n  \"rules\": {\n    \"basic.DeadCode\": \"off\",\n    \"EmptyCatchBlock\": {\"enabled\": false},\n    \"UnusedVariable\": {\"severity\": \"off\"},\n    \"UnusedImport\": {\"disabled\": true},\n    \"formatting.Indentation\": {\"spacesPerIndentLevel\": 2, \"severity\": \"info\"},\n    \"unnecessary.UnnecessaryGString\": \"warning\",\n    \"NoSuch\": {}\n  },\n  \"env\": {}\n}\n",
        );
        let (s, im) = states(&[f]);
        let t = im.toml();
        assert!(!s["DeadCode"]);
        assert!(!s["EmptyCatchBlock"]);
        assert!(!s["UnusedVariable"]);
        assert!(!s["UnusedImport"]);
        assert!(s["UnnecessaryGString"]);
        assert!(s["EmptyIfStatement"], "from recommended");
        assert!(t.contains("# indent = 2"), "{t}");
        assert!(t.contains("rules.NoSuch: not a CodeNarc"), "{t}");
        assert!(t.contains("env: not about rules"), "{t}");
        // jenkinsfile extends recommended and turns rules off
        let f = d.write(
            "j/.groovylintrc.json",
            "{\"extends\": [\"recommended-jenkinsfile\"]}",
        );
        let (s, _) = states(&[f]);
        assert!(!s["VariableName"]);
        assert!(s["DeadCode"]);
    }

    #[test]
    fn properties_adjust_the_rulesets() {
        let d = Dir::new("props");
        let x = d.write(
            "c.groovy",
            "ruleset {\n    ruleset('rulesets/basic.xml')\n}\n",
        );
        let p = d.write(
            "codenarc.properties",
            "# comment\nDeadCode.enabled=false\nEmptyCatchBlock.priority = 1\nUnusedImport.enabled=true\nDuplicateMapKey.doNotApplyToFileNames=a,\\\n  b\ncodenarc.other = x\n",
        );
        let (s, im) = states(&[p.clone(), x]);
        assert!(!s["DeadCode"]);
        assert!(!s.contains_key("UnusedImport"));
        let t = im.toml();
        assert!(t.contains("UnusedImport isn't in the rulesets"), "{t}");
        assert!(
            t.contains("DuplicateMapKey.doNotApplyToFileNames = a,b"),
            "{t}"
        );
        // alone, it sets only what it names
        let (s, im) = states(&[p]);
        assert_eq!(s.len(), 2, "{s:?}");
        let t = im.toml();
        assert!(
            t.contains("\nDeadCode = false\n") && t.contains("\nUnusedImport = true\n"),
            "{t}"
        );
        assert!(!t.contains("EmptyIfStatement"), "{t}");
    }

    #[test]
    fn output_is_deterministic() {
        let d = Dir::new("det");
        let f = d.write("c.groovy", "ruleset {\n    ruleset('rulesets/formatting.xml')\n    ruleset('rulesets/naming.xml')\n    ruleset('rulesets/basic.xml')\n}\n");
        let a = import(std::slice::from_ref(&f)).unwrap().toml();
        let b = import(&[f]).unwrap().toml();
        assert_eq!(a, b);
        assert!(a.contains("# normalize_spacing = true\n"), "{a}");
    }

    #[test]
    fn wildcards() {
        assert!(matches("Empty*", "EmptyCatchBlock"));
        assert!(matches("A, Empty?atchBlock", "EmptyCatchBlock"));
        assert!(!matches("Empty", "EmptyCatchBlock"));
        assert!(matches("*", "X"));
        assert!(matches("*Block", "EmptyCatchBlock"));
        assert!(!matches("Empty*Block", "EmptyIfStatement"));
        assert!(matches("E*p*y*", "Empty"));
        // a pattern that would take exponential time with naive backtracking
        let name = "a".repeat(60);
        assert!(!matches(&format!("{}b", "*a".repeat(30)), &name));
    }
}
