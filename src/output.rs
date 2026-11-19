//! What the command line prints: report lines, `--diff` patches, the JSON and SARIF
//! documents (through `report`), and fatal errors.

use crate::report::{self, Diagnostic, FileReport};
use crate::syntax::{self, SyntaxError};
use crate::{diff, display_path, lint, printable};
use std::io::Write;
use std::path::Path;
/// `--output-format`: report lines as text or as GitHub Actions workflow commands, or one
/// JSON or SARIF document on stdout instead.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum OutputFormat {
    #[default]
    Text,
    Github,
    Json,
    Sarif,
}

pub fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("groovyfmt: {}", printable(&msg.to_string()));
    std::process::exit(2);
}

/// One line to stdout. Write errors are ignored, so `groovyfmt --check | head -1` stops
/// printing quietly instead of panicking; the exit status still reflects the results.
pub fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// A path for display. A leading `:` gets `./` in front, so no report line can start
/// with `::` and be taken as a GitHub Actions workflow command.
pub fn shown(p: &Path) -> String {
    let s = printable(&display_path(p));
    // after any leading whitespace too, in case the log reader trims it
    if s.trim_start().starts_with(':') {
        format!("./{s}")
    } else {
        s
    }
}

/// Where a file failed to parse: the first places, and how many there are in all.
pub type Diag = (Vec<SyntaxError>, usize);

/// `diag` for a skipped file whose reasons are listed (`-v`, `--strict`), else nothing.
pub fn diag_if(listed: bool, src: &str) -> Diag {
    if listed {
        syntax::check(src)
    } else {
        Diag::default()
    }
}

/// One line per place a file failed to parse, and a count of any not shown. To stdout for
/// `--syntax`, else to stderr below the file's `skipped` line.
pub fn report_syntax(name: &str, (shown_errs, total): &Diag, github: bool, out: fn(&str)) {
    for e in shown_errs {
        let message = printable(&e.message);
        out(&if github {
            let at = Some((e.line, e.col));
            report::github("warning", name, at, Some("could not parse"), &message)
        } else {
            format!("{name}:{}:{}: could not parse: {message}", e.line, e.col)
        });
    }
    let more = total - shown_errs.len();
    if more > 0 {
        out(&if github {
            let message = format!("{more} more places not shown");
            report::github("warning", name, None, Some("could not parse"), &message)
        } else {
            format!("{name}: {more} more places groovyfmt could not parse")
        });
    }
}

pub fn to_stderr(line: &str) {
    eprintln!("{line}");
}

/// Text to stdout as it is, errors ignored as in `say`.
pub fn print_raw(s: &str) {
    let _ = std::io::stdout().lock().write_all(s.as_bytes());
}

/// `--diff`: the patch turning `old` into `new`, or a `#` line saying why it isn't shown.
/// A patch has to print the file name and content as they are, so it is withheld when
/// either could act on a terminal, a CI log or the patch itself:
/// - the name goes into the `---` / `+++` headers before anything else is looked at: any
///   control character there (a line break would start a header of its own, naming
///   another file), or a `"` or `\`, which git would quote, withholds it;
/// - the patch is then scanned as a whole, not line by line (splitting into lines would
///   hide a CR): no control character but a tab, a line feed, and a CR right before a line
///   feed (CR LF files diff as they are; that CR ends a line, so nothing can follow it on
///   that line); no `##[` anywhere; no line starting `::` after leading whitespace.
///
/// `git apply` and `patch` skip a `#` line between files, so the rest still applies.
pub fn patch_for(path: &Path, old: &str, new: &str) -> String {
    let name = display_path(path);
    let withheld = || {
        format!(
            "# groovyfmt: {}: diff not shown, as the file name or a changed line holds a control character or a CI workflow command\n",
            shown(path)
        )
    };
    if name
        .chars()
        .any(|c| c.is_control() || c == '"' || c == '\\')
    {
        return withheld();
    }
    let patch = diff::unified(&name, old, new);
    let mut chars = patch.chars().peekable();
    while let Some(c) = chars.next() {
        let crlf = c == '\r' && chars.peek() == Some(&'\n');
        if c.is_control() && !matches!(c, '\t' | '\n') && !crlf {
            return withheld();
        }
    }
    if patch.contains("##[") || patch.split('\n').any(|l| l.trim_start().starts_with("::")) {
        withheld()
    } else {
        patch
    }
}

/// The reason given for a file that doesn't parse.
pub const PARSE_ERROR: &str = "parse error";

pub fn parse_diagnostics((errs, _): Diag) -> Vec<Diagnostic> {
    errs.into_iter()
        .map(|e| Diagnostic {
            rule: report::PARSE_RULE,
            message: e.message,
            line: e.line,
            col: e.col,
            end_line: e.end_line,
            end_col: e.end_col,
            fixable: false,
        })
        .collect()
}

pub fn lint_diagnostic(v: lint::Violation) -> Diagnostic {
    Diagnostic {
        rule: v.rule,
        message: v.message,
        line: v.line,
        col: v.col,
        end_line: v.end_line,
        end_col: v.end_col,
        fixable: v.fix.is_some(),
    }
}

/// Where a run's results go. As text (or GitHub lines), each mode prints its own lines as it
/// goes and the summary line here; with `--output-format json` or `sarif`, the files are
/// collected and printed as one document on stdout instead, with no summary line (errors
/// still go to stderr).
pub struct Out {
    format: OutputFormat,
    files: Vec<FileReport>,
    /// More counts for the JSON summary
    pub extra: Vec<(&'static str, usize)>,
}

impl Out {
    pub fn new(format: OutputFormat) -> Out {
        Out {
            format,
            files: Vec::new(),
            extra: Vec::new(),
        }
    }

    /// Report lines, as text or GitHub workflow commands, rather than one document.
    pub fn text(&self) -> bool {
        matches!(self.format, OutputFormat::Text | OutputFormat::Github)
    }

    pub fn file(
        &mut self,
        path: &Path,
        status: &'static str,
        reason: Option<&str>,
        diagnostics: Vec<Diagnostic>,
    ) {
        if !self.text() {
            self.files.push(FileReport {
                path: display_path(path),
                status,
                reason: reason.map(String::from),
                diagnostics,
            });
        }
    }

    /// `e` names the file first (`read` errors) or not (write errors); the record doesn't.
    pub fn error(&mut self, path: &Path, e: &str) {
        let e = e.strip_prefix(&format!("{}: ", shown(path))).unwrap_or(e);
        self.file(path, "error", Some(e), Vec::new());
    }

    pub fn summary(self, line: &str) {
        let doc = match self.format {
            OutputFormat::Text | OutputFormat::Github => return eprintln!("{line}"),
            OutputFormat::Json => report::json_with(&self.files, &self.extra),
            OutputFormat::Sarif => report::sarif(&self.files, env!("CARGO_PKG_VERSION")),
        };
        let _ = std::io::stdout().lock().write_all(doc.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_neutralizes_line_breaks() {
        assert_eq!(
            printable("a\n::set-env name=X::y\r"),
            "a\\n::set-env name=X::y\\r"
        );
        assert_eq!(printable("plain é"), "plain é");
    }

    #[test]
    fn printable_neutralizes_legacy_workflow_commands() {
        assert_eq!(
            printable("x##[error]y ###[add-mask]z ##[##[w"),
            "x##\\[error]y ###\\[add-mask]z ##\\[##\\[w"
        );
        assert_eq!(printable("#\n#[x"), "#\\n#[x");
        for s in ["a##[b", "##[stop-commands]t%:,\r"] {
            let p = printable(s);
            for out in [
                shown(Path::new(s)),
                report::github("warning", &p, None, Some("t"), &p),
            ] {
                assert!(!out.contains("##["), "{out}");
            }
        }
    }

    #[test]
    fn shown_never_starts_a_line_with_a_colon() {
        assert_eq!(
            shown(Path::new("::warning title=x::y.groovy")),
            "./::warning title=x::y.groovy"
        );
        assert_eq!(shown(Path::new("src/A.groovy")), "src/A.groovy");
        assert_eq!(shown(Path::new(" \u{a0}::x.groovy")), "./ \u{a0}::x.groovy");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_print_with_forward_slashes() {
        assert_eq!(shown(Path::new(r"src\a\A.groovy")), "src/a/A.groovy");
        assert_eq!(shown(Path::new(r"C:\x\A.groovy")), "C:/x/A.groovy");
        let p = patch_for(Path::new(r"src\A.groovy"), "x  \n", "x\n");
        assert!(
            p.starts_with("--- a/src/A.groovy\n+++ b/src/A.groovy\n"),
            "{p}"
        );
        // an unpaired surrogate can't be printed; it shows as U+FFFD
        use std::os::windows::ffi::OsStringExt;
        let bad = std::path::PathBuf::from(std::ffi::OsString::from_wide(&[0x61, 0xD800, 0x62]));
        assert_eq!(shown(&bad), "a\u{FFFD}b");
    }

    #[test]
    fn patches_are_withheld_for_unsafe_names_and_content() {
        const UGLY: &str = "class A {\ndef f() {}\n}\n";
        const PRETTY: &str = "class A {\n    def f() {}\n}\n";
        let withheld = |p: &str| patch_for(Path::new(p), UGLY, PRETTY).starts_with("# groovyfmt: ");
        // a name is checked before it reaches a header
        for name in [
            "x\n+++ b/other.groovy",
            "x\r\n+++ b/other.groovy",
            "a\rb.groovy",
            "a\tb.groovy",
            "a\x1b[31m.groovy",
            "a\u{85}b.groovy",
            "q\"x.groovy",
            "b\\s.groovy",
            "x##[error]y.groovy",
        ]
        .into_iter()
        // on Windows `\` separates directories and prints as `/` (windows_paths_print_with_forward_slashes)
        .filter(|n| cfg!(unix) || !n.contains('\\'))
        {
            assert!(withheld(name), "{name:?}");
            let out = patch_for(Path::new(name), UGLY, PRETTY);
            assert_eq!(out.lines().count(), 1, "{name:?}: one notice line");
            assert!(
                out.starts_with("# groovyfmt: ") && !out.contains("##["),
                "{out:?}"
            );
        }
        // a leading `::` in a name lands after `--- a/`, never at a line start
        let p = patch_for(Path::new("::warning::x.groovy"), UGLY, PRETTY);
        assert!(p.starts_with("--- a/::warning::x.groovy\n"), "{p}");
        assert!(p.split('\n').all(|l| !l.trim_start().starts_with("::")));
        assert!(!withheld("src/A b-c.groovy"));

        // content: CR LF is kept as it is; a CR anywhere else, ESC, `##[` or a `::` line is not
        let crlf = patch_for(
            Path::new("A.groovy"),
            "class A {\r\ndef f() {}\r\n}\r\n",
            "class A {\r\n    def f() {}\r\n}\r\n",
        );
        assert!(
            crlf.contains("\n-def f() {}\r\n+    def f() {}\r\n"),
            "{crlf:?}"
        );
        for line in [
            "def s = 'a\rb'",
            "def s = '\x1b[2J'",
            "def s = '##[error]'",
            "// x\n::set-output x",
        ] {
            let old = format!("class A {{\n{line}\ndef f() {{}}\n}}\n");
            let new = format!("class A {{\n{line}\n    def f() {{}}\n}}\n");
            assert!(
                patch_for(Path::new("A.groovy"), &old, &new).starts_with("# groovyfmt: "),
                "{line:?}"
            );
        }
    }
}
