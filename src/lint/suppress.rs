//! Suppressing reports from the source: comments and `@SuppressWarnings`, applied to a file's
//! violations inside `lint_file`, so the CLI, `--fix` and the language server all see the same
//! result.
//!
//! - `// groovyfmt-ignore [Rule, ...]` on a line with code: that line; alone on its line: the
//!   next line.
//! - `// groovyfmt-ignore-start [Rule, ...]` up to `// groovyfmt-ignore-end` (or the end of
//!   the file).
//! - CodeNarc's own: `codenarc-disable [Rule, ...]` up to `codenarc-enable [Rule, ...]` (or the
//!   end of the file), `codenarc-disable-line [Rule, ...]` for its line, and
//!   `@SuppressWarnings('Rule')` / `@SuppressWarnings(['A', 'B'])` on a class, method,
//!   constructor, field or variable for that declaration (on an import: the whole file),
//!   with `'all'` or `'CodeNarc'` for every rule.
//!
//! Without a rule list a directive covers every rule. Either comment form works (`//` or
//! `/* */`). A report is suppressed by where it starts.

use super::{RULES, Violation, at};
use crate::text;
use tree_sitter::Node;

/// The node kinds `apply` looks at.
pub(super) const KINDS: &[&str] = &["line_comment", "block_comment", "annotation"];

/// The rules a directive names: all of them, or a list with whether each suppressed
/// anything.
enum Rules {
    All(bool),
    Some(Vec<(String, bool)>),
}

impl Rules {
    /// `A, B -- why`: the names before an optional ` -- ` and the reason after it.
    fn parse(list: &str) -> Rules {
        let list = list.split(" -- ").next().unwrap_or("");
        let names: Vec<(String, bool)> = list
            .split([',', ' ', '\t'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| (s.to_string(), false))
            .collect();
        if names.is_empty() {
            Rules::All(false)
        } else {
            Rules::Some(names)
        }
    }

    /// Whether this covers `rule`, marking the name used if so.
    fn take(&mut self, rule: &str) -> bool {
        match self {
            Rules::All(used) => {
                *used = true;
                true
            }
            Rules::Some(names) => names.iter_mut().any(|(n, used)| {
                let hit = n == rule;
                *used |= hit;
                hit
            }),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Family {
    Groovyfmt,
    Codenarc,
    Annotation,
}

/// One directive: the lines it covers (1-based, inclusive) and its rules.
struct Directive<'a> {
    node: Node<'a>,
    family: Family,
    from: usize,
    to: usize,
    rules: Rules,
}

pub(super) fn apply(src: &str, nodes: &[Node], v: &mut Vec<Violation>) {
    let mut ds = directives(src, nodes);
    if ds.is_empty() {
        return;
    }
    v.retain(|x| {
        // every directive covering the report counts as used, so none is called unused
        // because an outer one got there first
        let mut hit = false;
        for d in ds.iter_mut() {
            if (d.from..=d.to).contains(&x.line) {
                hit |= d.rules.take(x.rule);
            }
        }
        !hit
    });
    for d in &ds {
        let unused: Vec<&str> = match &d.rules {
            // a bare `codenarc-disable` or `@SuppressWarnings('all')` serves CodeNarc and
            // the compiler too, so only groovyfmt's own are judged
            Rules::All(false) if d.family == Family::Groovyfmt => vec!["any rule"],
            Rules::All(_) => vec![],
            Rules::Some(names) => names
                .iter()
                .filter(|(n, used)| {
                    !used
                        && (d.family == Family::Groovyfmt
                            || RULES.iter().any(|r| r.name == n))
                })
                .map(|(n, _)| n.as_str())
                .collect(),
        };
        if !unused.is_empty() {
            v.push(at(
                d.node,
                "UnusedSuppression",
                format!(
                    "This suppression of {} matches no report; remove it (or fix the rule name)",
                    unused.join(", ")
                ),
            ));
        }
    }
}

/// The comment's text without its markers.
fn body<'a>(n: Node, src: &'a str) -> &'a str {
    let t = text(n, src);
    t.strip_prefix("//")
        .or_else(|| t.strip_prefix("/*").map(|b| b.strip_suffix("*/").unwrap_or(b)))
        .unwrap_or(t)
        .trim()
}

/// The directive word a comment starts with, and the rest.
fn directive<'a>(b: &'a str) -> Option<(&'a str, &'a str)> {
    const WORDS: &[&str] = &[
        "groovyfmt-ignore-start",
        "groovyfmt-ignore-end",
        "groovyfmt-ignore",
        "codenarc-disable-line",
        "codenarc-disable",
        "codenarc-enable",
    ];
    WORDS.iter().find_map(|w| {
        let rest = b.strip_prefix(w)?;
        // a whole word: `groovyfmt-ignored` is prose
        (rest.is_empty() || rest.starts_with([' ', '\t', ','])).then_some((*w, rest))
    })
}

fn directives<'a>(src: &str, nodes: &[Node<'a>]) -> Vec<Directive<'a>> {
    let last = src.split('\n').count();
    let mut out: Vec<Directive> = Vec::new();
    // open regions, by family: indexes into `out`
    let mut open: Vec<usize> = Vec::new();
    for &n in nodes {
        let line = n.start_position().row + 1;
        if n.kind() == "annotation" {
            if let Some(d) = suppress_warnings(src, n, last) {
                out.push(d);
            }
            continue;
        }
        let Some((word, rest)) = directive(body(n, src)) else {
            continue;
        };
        let rules = Rules::parse(rest);
        let (family, from, to) = match word {
            "groovyfmt-ignore" => {
                let alone = src[..n.start_byte()]
                    .rsplit('\n')
                    .next()
                    .is_some_and(|before| before.trim().is_empty());
                let l = if alone { line + 1 } else { line };
                (Family::Groovyfmt, l, l)
            }
            "codenarc-disable-line" => (Family::Codenarc, line, line),
            "groovyfmt-ignore-start" => (Family::Groovyfmt, line, last),
            "codenarc-disable" => (Family::Codenarc, line, last),
            _ => {
                // an end: closes the open regions of its family (those naming its rules,
                // when it names any)
                let family = if word == "groovyfmt-ignore-end" {
                    Family::Groovyfmt
                } else {
                    Family::Codenarc
                };
                open.retain(|&i| {
                    let d = &mut out[i];
                    let closes = d.family == family
                        && match (&rules, &d.rules) {
                            (Rules::All(_), _) => true,
                            (Rules::Some(names), Rules::Some(theirs)) => {
                                names.iter().any(|(n, _)| theirs.iter().any(|(t, _)| t == n))
                            }
                            (Rules::Some(_), Rules::All(_)) => false,
                        };
                    if closes {
                        d.to = line;
                    }
                    !closes
                });
                continue;
            }
        };
        if matches!(word, "groovyfmt-ignore-start" | "codenarc-disable") {
            open.push(out.len());
        }
        out.push(Directive {
            node: n,
            family,
            from,
            to,
            rules,
        });
    }
    out
}

/// `@SuppressWarnings('A')` / `@SuppressWarnings(['A', 'B'])`, covering the declaration it
/// annotates (an import: the whole file).
fn suppress_warnings<'a>(src: &str, n: Node<'a>, last: usize) -> Option<Directive<'a>> {
    let name = text(n.child_by_field_name("name")?, src);
    if name.rsplit('.').next() != Some("SuppressWarnings") {
        return None;
    }
    let args = n.child_by_field_name("arguments")?;
    let mut names = Vec::new();
    crate::preorder(args, |x| {
        if x.kind() == "string_literal" {
            let t = text(x, src).trim_matches(['\'', '"']);
            names.push(t.to_string());
            return false;
        }
        true
    });
    let all = names.iter().any(|x| x == "all" || x == "CodeNarc");
    let rules = if all {
        Rules::All(false)
    } else {
        Rules::Some(names.into_iter().map(|x| (x, false)).collect())
    };
    let decl = n.parent()?;
    let (from, to) = if decl.kind() == "import_declaration" {
        (1, last)
    } else if decl.kind().ends_with("_declaration") {
        (decl.start_position().row + 1, decl.end_position().row + 1)
    } else {
        return None;
    };
    Some(Directive {
        node: n,
        family: Family::Annotation,
        from,
        to,
        rules,
    })
}

#[cfg(test)]
mod tests {
    use crate::lint::lint;

    fn rules(src: &str) -> Vec<(&'static str, usize)> {
        lint(src)
            .expect("parses")
            .into_iter()
            .map(|v| (v.rule, v.line))
            .collect()
    }

    #[test]
    fn ignore_comments_cover_their_line_or_the_next() {
        let src = "import a.B // groovyfmt-ignore\n// groovyfmt-ignore UnusedImport\nimport a.C\nimport a.D // groovyfmt-ignore DuplicateImport\nx = 1; // groovyfmt-ignore UnusedImport, UnnecessarySemicolon -- generated\ny = 2;\n";
        assert_eq!(
            rules(src),
            vec![
                ("UnusedImport", 4),
                ("UnusedSuppression", 4),
                ("UnusedSuppression", 5),
                ("UnnecessarySemicolon", 6)
            ]
        );
        // block comments too; prose that merely starts with the word is no directive
        let src = "import a.B /* groovyfmt-ignore */\nimport a.C // groovyfmt-ignored by us\n";
        assert_eq!(rules(src), vec![("UnusedImport", 2)]);
    }

    #[test]
    fn regions() {
        let src = "// groovyfmt-ignore-start UnusedImport\nimport a.B\nx = 1;\n// groovyfmt-ignore-end\nimport a.C\n// groovyfmt-ignore-start\ny = 2;\n";
        assert_eq!(
            rules(src),
            vec![("UnnecessarySemicolon", 3), ("UnusedImport", 5)]
        );
    }

    #[test]
    fn codenarc_comments() {
        let src = "// codenarc-disable UnusedImport, NoDef\nimport a.B\n// codenarc-enable UnusedImport, NoDef\nimport a.C\nx = 1; /* codenarc-disable-line */\n/* codenarc-disable */\ny = 2;\n/* codenarc-enable */\nz = 3;\n";
        assert_eq!(
            rules(src),
            vec![("UnusedImport", 4), ("UnnecessarySemicolon", 9)]
        );
    }

    #[test]
    fn suppress_warnings_annotations() {
        let src = "@SuppressWarnings('UnusedPrivateField')\nclass A {\n  private int a\n}\nclass B {\n  @SuppressWarnings(['UnusedPrivateField', 'unchecked'])\n  private int b\n  private int c\n}\n@SuppressWarnings('all')\nclass C {\n  private int d\n}\n@SuppressWarnings('CodeNarc')\nclass D {\n  private int e\n}\n";
        assert_eq!(rules(src), vec![("UnusedPrivateField", 8)]);
        // on an import: the whole file
        let src = "@SuppressWarnings('UnusedImport')\nimport a.B\nimport a.C\n";
        assert_eq!(rules(src), vec![]);
    }

    #[test]
    fn unused_suppressions() {
        let src = "// groovyfmt-ignore\nprintln 1\nprintln 2 // groovyfmt-ignore UnusedImport, NoSuchRule\n// codenarc-disable-line UnusedImport, Println\n@SuppressWarnings(['unchecked', 'UnusedPrivateField'])\nclass A { }\n@SuppressWarnings('all')\nclass B { }\n";
        let v = lint(src).unwrap();
        let got: Vec<_> = v.iter().map(|v| (v.rule, v.line)).collect();
        assert_eq!(
            got,
            vec![
                ("UnusedSuppression", 1),
                ("UnusedSuppression", 3),
                ("UnusedSuppression", 4),
                ("UnusedSuppression", 5)
            ]
        );
        assert!(v[1].message.contains("UnusedImport, NoSuchRule"), "{}", v[1].message);
        assert!(v[2].message.contains("of UnusedImport matches"), "{}", v[2].message);
    }
}
