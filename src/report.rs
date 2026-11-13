//! Machine-readable reports: the JSON document of `--output-format json` (schema in
//! docs/output-formats.md, versioned by `JSON_VERSION`) and the SARIF 2.1.0 log of `--output-format sarif`.
//! Written by hand to stay free of dependencies; every string goes through `string`.

use crate::lint::RULES;
use std::fmt::Write;

/// Bumped on any change to the JSON document that could break a reader.
pub const JSON_VERSION: u32 = 1;

/// The rule name of a place a file could not be parsed.
pub const PARSE_RULE: &str = "ParseError";

const PARSE_RULE_DESCRIPTION: &str = "a place groovyfmt could not parse; the file was not formatted or linted (the grammar is not the compiler, so valid code can fail too)";

pub struct Diagnostic {
    pub rule: &'static str,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub fixable: bool,
}

pub struct FileReport {
    /// As the text output names the file, unescaped.
    pub path: String,
    /// unchanged, would_reformat, reformatted, fixed, skipped or error
    pub status: &'static str,
    /// Why the file was skipped, the error, or (`--fix`) why it was not fixed.
    pub reason: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// A JSON string literal. Every control character (C0, DEL and C1, which terminals act on)
/// is escaped, and so is the `[` of `##[`, which the GitHub Actions runner takes as a
/// workflow command anywhere in a log line. All of it decodes back to the original text.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut hashes = 0;
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '[' if hashes >= 2 => out.push_str("\\u005b"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
        hashes = if c == '#' { hashes + 1 } else { 0 };
    }
    out.push('"');
    out
}

fn opt(s: Option<&str>) -> String {
    s.map_or_else(|| "null".into(), string)
}

/// The `--output-format json` document: one line per file between the head and the summary.
pub fn json(files: &[FileReport]) -> String {
    json_with(files, &[])
}

/// `json`, with more counts at the end of the summary (`--lint` with a baseline).
pub fn json_with(files: &[FileReport], extra: &[(&str, usize)]) -> String {
    let mut s = format!("{{\"version\":{JSON_VERSION},\"files\":[");
    let count = |st: &str| files.iter().filter(|f| f.status == st).count();
    let mut diagnostics = 0;
    for (i, f) in files.iter().enumerate() {
        s.push_str(if i == 0 { "\n" } else { ",\n" });
        let _ = write!(
            s,
            "{{\"path\":{},\"status\":{},\"skip_reason\":{},\"diagnostics\":[",
            string(&f.path),
            string(f.status),
            opt(f.reason.as_deref())
        );
        for (j, d) in f.diagnostics.iter().enumerate() {
            diagnostics += 1;
            if j > 0 {
                s.push(',');
            }
            let _ = write!(
                s,
                "{{\"rule\":{},\"severity\":\"warning\",\"message\":{},\"line\":{},\"column\":{},\"end_line\":{},\"end_column\":{},\"fixable\":{}}}",
                string(d.rule),
                string(&d.message),
                d.line,
                d.col,
                d.end_line,
                d.end_col,
                d.fixable
            );
        }
        s.push_str("]}");
    }
    let extra: String = extra
        .iter()
        .map(|(k, n)| format!(",{}:{n}", string(k)))
        .collect();
    let _ = writeln!(
        s,
        "\n],\"summary\":{{\"files\":{},\"unchanged\":{},\"would_reformat\":{},\"reformatted\":{},\"fixed\":{},\"skipped\":{},\"errors\":{},\"diagnostics\":{diagnostics}{extra}}}}}",
        files.len(),
        count("unchanged"),
        count("would_reformat"),
        count("reformatted"),
        count("fixed"),
        count("skipped"),
        count("error"),
    );
    s
}

/// A path as a SARIF `artifactLocation.uri`, a relative (or absolute-path) URI reference:
/// every byte other than unreserved characters and `/` percent-encoded.
fn uri(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The `--output-format sarif` log: lint violations and parse errors as results, for
/// GitHub code scanning (`github/codeql-action/upload-sarif`) and other SARIF readers.
pub fn sarif(files: &[FileReport], version: &str) -> String {
    let rules: Vec<(&str, &str, bool)> = RULES
        .iter()
        .map(|r| (r.name, r.description, r.default))
        .chain([(PARSE_RULE, PARSE_RULE_DESCRIPTION, true)])
        .collect();
    let mut s = String::from(
        "{\"$schema\":\"https://json.schemastore.org/sarif-2.1.0.json\",\"version\":\"2.1.0\",\"runs\":[{\"tool\":{\"driver\":{\"name\":\"groovyfmt\",",
    );
    let _ = write!(
        s,
        "\"version\":{},\"informationUri\":\"https://github.com/costajohnt/groovyfmt\",\"rules\":[",
        string(version)
    );
    for (i, (id, description, on)) in rules.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(
            s,
            "\n{{\"id\":{},\"shortDescription\":{{\"text\":{}}},\"defaultConfiguration\":{{\"level\":\"warning\",\"enabled\":{on}}}}}",
            string(id),
            string(description)
        );
    }
    s.push_str("]}},\"columnKind\":\"unicodeCodePoints\",\"results\":[");
    let mut first = true;
    for f in files {
        for d in &f.diagnostics {
            let index = rules.iter().position(|r| r.0 == d.rule).unwrap_or(0);
            s.push_str(if first { "\n" } else { ",\n" });
            first = false;
            let _ = write!(
                s,
                "{{\"ruleId\":{},\"ruleIndex\":{index},\"level\":\"warning\",\"message\":{{\"text\":{}}},\"locations\":[{{\"physicalLocation\":{{\"artifactLocation\":{{\"uri\":{}}},\"region\":{{\"startLine\":{},\"startColumn\":{},\"endLine\":{},\"endColumn\":{}}}}}}}]}}",
                string(d.rule),
                string(&d.message),
                string(&uri(&f.path)),
                d.line,
                d.col,
                d.end_line,
                d.end_col
            );
        }
    }
    s.push_str("\n]}]}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_escape_quotes_controls_and_workflow_commands() {
        assert_eq!(string("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(
            string("\n\r\t\x00\x1b\x7f\u{9b}"),
            r#""\n\r\t\u0000\u001b\u007f\u009b""#
        );
        assert_eq!(string("é ✓ 😀"), "\"é ✓ 😀\"");
        // `[` is `[`
        let lb = "\\u005b";
        assert_eq!(
            string("x##[error]##[#[ ###["),
            format!("\"x##{lb}error]##{lb}#[ ###{lb}\"")
        );
        assert!(!string("##[a]").contains("##["));
    }

    #[test]
    fn uris_percent_encode_everything_but_unreserved_and_slashes() {
        assert_eq!(uri("src/A-b_c.~1.groovy"), "src/A-b_c.~1.groovy");
        assert_eq!(uri("a b/%#?é.groovy"), "a%20b/%25%23%3F%C3%A9.groovy");
    }
}
