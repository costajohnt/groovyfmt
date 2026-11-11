//! End-to-end tests against the built binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const UGLY: &str = "class A {\ndef f() {\nprintln 'x'   \n}\n}\n";
const PRETTY: &str = "class A {\n    def f() {\n        println 'x'\n    }\n}\n";

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("groovyfmt-cli-{name}-{}", std::process::id()));
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
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.0.join(rel)).unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_groovyfmt"));
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    // A child that exits early (a bad argument) closes its stdin before the write
    // finishes; that's fine, its exit status and stderr are what the tests check.
    if let Err(e) = child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.unwrap_or("").as_bytes())
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        panic!("writing the child's stdin: {e}");
    }
    child.wait_with_output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn version_and_help() {
    let d = Dir::new("version");
    let v = run(&d.0, &["--version"], None);
    assert!(v.status.success());
    assert_eq!(
        stdout(&v),
        format!("groovyfmt {}\n", env!("CARGO_PKG_VERSION"))
    );
    let h = run(&d.0, &["--help"], None);
    assert!(h.status.success() && stdout(&h).contains("Usage: groovyfmt"));
}

#[test]
fn check_then_write_then_check() {
    let d = Dir::new("write");
    d.write("src/A.groovy", UGLY);
    d.write("build.gradle", "plugins {\n  id 'java'\n}\n");
    let c = run(&d.0, &["--check"], None);
    assert_eq!(c.status.code(), Some(1));
    assert_eq!(
        stdout(&c),
        "build.gradle\nsrc/A.groovy\n",
        "sorted, no ./ prefix"
    );
    assert_eq!(d.read("src/A.groovy"), UGLY, "--check must not write");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = d.0.join("src/A.groovy");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let w = run(&d.0, &[], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(d.read("src/A.groovy"), PRETTY);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(d.0.join("src/A.groovy"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o640,
            "permissions kept across the atomic rewrite"
        );
    }
    let leftovers: Vec<_> = std::fs::read_dir(d.0.join("src"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".groovyfmt-"))
        .collect();
    assert!(leftovers.is_empty(), "no temp files left behind");
    assert_eq!(d.read("build.gradle"), "plugins {\n    id 'java'\n}\n");
    assert_eq!(run(&d.0, &["--check"], None).status.code(), Some(0));
}

#[test]
fn skips_build_and_hidden_dirs_but_not_explicit_files() {
    let d = Dir::new("skip");
    d.write("build/gen/G.groovy", UGLY);
    d.write(".hidden/H.groovy", UGLY);
    d.write("node_modules/x/N.groovy", UGLY);
    d.write("notes.txt", "not groovy");
    assert_eq!(run(&d.0, &["--check"], None).status.code(), Some(0));
    let explicit = run(&d.0, &["--check", "build/gen/G.groovy"], None);
    assert_eq!(explicit.status.code(), Some(1));
}

#[test]
fn unparseable_files_are_left_alone() {
    let d = Dir::new("broken");
    d.write("Broken.groovy", "def x = {\n");
    let w = run(&d.0, &["-v"], None);
    assert_eq!(w.status.code(), Some(0));
    assert_eq!(d.read("Broken.groovy"), "def x = {\n");
    assert!(stderr(&w).contains("skipped") && stderr(&w).contains("parse error"));
}

#[test]
fn strict_fails_on_skipped_files_and_lists_them() {
    let d = Dir::new("strict");
    d.write("Broken.groovy", "def x = {\n");
    d.write("Ok.groovy", PRETTY);
    for mode in [&["--check"][..], &["--lint"], &[]] {
        let plain = run(&d.0, mode, None);
        assert_eq!(plain.status.code(), Some(0), "{mode:?}");
        assert!(!stderr(&plain).contains("Broken.groovy: parse error"));
        let args: Vec<&str> = mode.iter().copied().chain(["--strict"]).collect();
        let strict = run(&d.0, &args, None);
        assert_eq!(strict.status.code(), Some(1), "{mode:?}");
        assert!(
            stderr(&strict).contains("skipped Broken.groovy: parse error"),
            "{mode:?}: {}",
            stderr(&strict)
        );
    }
    std::fs::remove_file(d.0.join("Broken.groovy")).unwrap();
    assert_eq!(
        run(&d.0, &["--check", "--strict"], None).status.code(),
        Some(0)
    );
}

const BROKEN_AT: &str =
    "Broken.groovy:1:10: could not parse: expected `}` to close the `{` on line 1\n";

#[test]
fn syntax_mode_reports_where_parsing_failed() {
    let d = Dir::new("syntax");
    d.write("Ok.groovy", PRETTY);
    let o = run(&d.0, &["--syntax"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        (stdout(&o), stderr(&o)),
        ("".into(), "could not parse 0 of 1 files\n".into())
    );

    d.write("Broken.groovy", "def x = {\n");
    let o = run(&d.0, &["--syntax"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stdout(&o), BROKEN_AT);
    assert_eq!(stderr(&o), "could not parse 1 of 2 files\n");
    let g = run(&d.0, &["--syntax", "--output-format", "github"], None);
    assert_eq!(g.status.code(), Some(1));
    assert_eq!(
        stdout(&g),
        "::warning file=Broken.groovy,line=1,col=10,title=could not parse::expected `}` to close the `{` on line 1\n"
    );
    assert_eq!(d.read("Broken.groovy"), "def x = {\n", "never writes");

    // at most ten places per file, then a count of the rest
    let many: String = (0..12).map(|i| format!("def f{i}() {{ x = }}\n")).collect();
    d.write("many/M.groovy", &many);
    let o = run(&d.0, &["--syntax", "many"], None);
    assert_eq!(o.status.code(), Some(1));
    let lines: Vec<String> = stdout(&o).lines().map(String::from).collect();
    assert_eq!(lines.len(), 11, "{lines:?}");
    assert_eq!(
        lines[0],
        "many/M.groovy:1:14: could not parse: unexpected `=` in a block"
    );
    assert_eq!(
        lines[10],
        "many/M.groovy: 2 more places groovyfmt could not parse"
    );

    for (args, msg) in [
        (&["--syntax", "--check"][..], "can't be combined"),
        (
            &["--syntax", "nope.groovy"],
            "nope.groovy: no such file or directory",
        ),
    ] {
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(stderr(&o).contains(msg), "{args:?}: {}", stderr(&o));
    }
}

#[test]
fn skipped_files_show_where_parsing_failed() {
    let d = Dir::new("syntaxskip");
    d.write("Broken.groovy", "def x = {\n");
    for mode in [&["--check"][..], &["--lint"], &[]] {
        for flag in ["-v", "--strict"] {
            let args: Vec<&str> = mode.iter().copied().chain([flag]).collect();
            let o = run(&d.0, &args, None);
            assert!(
                stderr(&o).contains(&format!("skipped Broken.groovy: parse error\n{BROKEN_AT}")),
                "{args:?}: {}",
                stderr(&o)
            );
        }
        let plain = run(&d.0, mode, None);
        assert!(!stderr(&plain).contains("could not parse"), "{mode:?}");
    }
    let o = run(&d.0, &["--stdin"], Some("def x = {\n"));
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(stdout(&o), "");
    assert_eq!(
        stderr(&o),
        "groovyfmt: <stdin>: not formatted: parse error\n<stdin>:1:10: could not parse: expected `}` to close the `{` on line 1\n"
    );
    let args = [
        "--stdin",
        "--stdin-filepath",
        "a:b.groovy",
        "--output-format",
        "github",
    ];
    let o = run(&d.0, &args, Some("def x = {\n"));
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(
        stderr(&o),
        "::error file=a%3Ab.groovy::Not formatted: parse error\n::warning file=a%3Ab.groovy,line=1,col=10,title=could not parse::expected `}` to close the `{` on line 1\n"
    );
}

#[cfg(unix)]
#[test]
fn syntax_messages_escape_names_and_quoted_source() {
    let d = Dir::new("syntaxesc");
    d.write("x##[error]\x1b[31m.groovy", "f(a b '##[e]\x07')\n");
    let o = run(&d.0, &["--syntax"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        "x##\\[error]\\u{1b}[31m.groovy:1:7: could not parse: unexpected `'##\\[e]\\u{7}'` in an argument list\n"
    );
    let g = run(&d.0, &["--syntax", "--output-format", "github"], None);
    assert!(
        stdout(&g)
            .lines()
            .all(|l| !l.contains("##[") && !l.contains('\x1b')),
        "{}",
        stdout(&g)
    );
    assert!(
        stdout(&g).starts_with("::warning file=x##\\[error]"),
        "{}",
        stdout(&g)
    );
}

const LINTED: &str = "import a.b.Unused\ndef a = 1;\n";

/// A, Broken and Ugly: lint violations, a parse error, and a file to format.
fn json_dir(name: &str) -> Dir {
    let d = Dir::new(name);
    d.write("A.groovy", LINTED);
    d.write("Broken.groovy", "def x = {\n");
    d.write("Ugly.groovy", UGLY);
    d
}

const JSON_A: &str = r#"{"path":"A.groovy","status":"unchanged","skip_reason":null,"diagnostics":[{"rule":"UnusedImport","severity":"warning","message":"The [a.b.Unused] import is never referenced","line":1,"column":1,"end_line":1,"end_column":18,"fixable":true},{"rule":"UnnecessarySemicolon","severity":"warning","message":"Semicolons as line endings can be removed safely","line":2,"column":10,"end_line":2,"end_column":11,"fixable":true}]}"#;
const JSON_BROKEN: &str = r#"{"path":"Broken.groovy","status":"skipped","skip_reason":"parse error","diagnostics":[{"rule":"ParseError","severity":"warning","message":"expected `}` to close the `{` on line 1","line":1,"column":10,"end_line":1,"end_column":10,"fixable":false}]}"#;

fn json_file(path: &str, status: &str) -> String {
    format!(r#"{{"path":"{path}","status":"{status}","skip_reason":null,"diagnostics":[]}}"#)
}

fn json_doc(files: &[&str], summary: &str) -> String {
    format!(
        "{{\"version\":1,\"files\":[\n{}\n],\"summary\":{{{summary}}}}}\n",
        files.join(",\n")
    )
}

#[test]
fn json_output_is_one_document_on_stdout() {
    let d = json_dir("json");
    let o = run(&d.0, &["--lint", "--output-format", "json"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stderr(&o), "", "no summary line");
    assert_eq!(
        stdout(&o),
        json_doc(
            &[JSON_A, JSON_BROKEN, &json_file("Ugly.groovy", "unchanged")],
            r#""files":3,"unchanged":2,"would_reformat":0,"reformatted":0,"fixed":0,"skipped":1,"errors":0,"diagnostics":3"#
        )
    );

    let o = run(&d.0, &["--check", "--output-format=json"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stderr(&o), "");
    let unchanged_a = json_file("A.groovy", "unchanged");
    assert_eq!(
        stdout(&o),
        json_doc(
            &[
                &unchanged_a,
                JSON_BROKEN,
                &json_file("Ugly.groovy", "would_reformat")
            ],
            r#""files":3,"unchanged":1,"would_reformat":1,"reformatted":0,"fixed":0,"skipped":1,"errors":0,"diagnostics":1"#
        )
    );
    assert_eq!(d.read("Ugly.groovy"), UGLY);

    let o = run(&d.0, &["--syntax", "--output-format", "json"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        json_doc(
            &[
                &unchanged_a,
                JSON_BROKEN,
                &json_file("Ugly.groovy", "unchanged")
            ],
            r#""files":3,"unchanged":2,"would_reformat":0,"reformatted":0,"fixed":0,"skipped":1,"errors":0,"diagnostics":1"#
        )
    );

    let o = run(&d.0, &["--output-format", "json", "-v"], None);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        stderr(&o),
        "config: none found, using defaults\n",
        "-v keeps its config lines on stderr"
    );
    assert_eq!(
        stdout(&o),
        json_doc(
            &[
                &unchanged_a,
                JSON_BROKEN,
                &json_file("Ugly.groovy", "reformatted")
            ],
            r#""files":3,"unchanged":1,"would_reformat":0,"reformatted":1,"fixed":0,"skipped":1,"errors":0,"diagnostics":1"#
        )
    );
    assert_eq!(d.read("Ugly.groovy"), PRETTY);

    // no files: still a document
    let e = Dir::new("jsonempty");
    let o = run(&e.0, &["--check", "--output-format", "json"], None);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        stdout(&o),
        "{\"version\":1,\"files\":[\n],\"summary\":{\"files\":0,\"unchanged\":0,\"would_reformat\":0,\"reformatted\":0,\"fixed\":0,\"skipped\":0,\"errors\":0,\"diagnostics\":0}}\n"
    );
}

#[cfg(unix)]
#[test]
fn json_records_unreadable_files_as_errors() {
    use std::os::unix::fs::PermissionsExt;
    let d = Dir::new("jsonerr");
    let p = d.write("Locked.groovy", PRETTY);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
    // root reads anything; nothing to test then
    if std::fs::read(&p).is_ok() {
        return;
    }
    let o = run(&d.0, &["--lint", "--output-format", "json"], None);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("Locked.groovy"), "{}", stderr(&o));
    assert!(
        stdout(&o).contains(
            r#"{"path":"Locked.groovy","status":"error","skip_reason":"Permission denied (os error 13)","diagnostics":[]}"#
        ),
        "{}",
        stdout(&o)
    );
}

#[test]
fn sarif_output_for_lint_and_parse_errors() {
    let d = json_dir("sarif");
    let o = run(&d.0, &["--lint", "--output-format", "sarif"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stderr(&o), "");
    let s = stdout(&o);
    assert!(
        s.starts_with(r#"{"$schema":"https://json.schemastore.org/sarif-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"groovyfmt","#),
        "{s}"
    );
    assert!(s.contains(r#"{"id":"UnusedImport","shortDescription":{"text":"import whose name (or alias) never appears in code or as a word in a comment"},"defaultConfiguration":{"level":"warning","enabled":true}}"#), "{s}");
    assert!(s.contains(r#""columnKind":"unicodeCodePoints""#), "{s}");
    assert!(s.contains(r#"{"ruleId":"UnusedImport","ruleIndex":0,"level":"warning","message":{"text":"The [a.b.Unused] import is never referenced"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"A.groovy"},"region":{"startLine":1,"startColumn":1,"endLine":1,"endColumn":18}}}]}"#), "{s}");
    // ParseError comes after every rule in the table
    let parse = format!(
        r#"{{"ruleId":"ParseError","ruleIndex":{},"level":"warning","message":{{"text":"expected `}}` to close the `{{` on line 1"}},"locations":[{{"physicalLocation":{{"artifactLocation":{{"uri":"Broken.groovy"}},"region":{{"startLine":1,"startColumn":10,"endLine":1,"endColumn":10}}}}}}]}}"#,
        groovyfmt::lint::RULES.len()
    );
    assert!(s.contains(&parse), "{s}");
    assert_eq!(s.matches("\"ruleId\"").count(), 3);

    for (args, msg) in [
        (
            &["--check", "--output-format", "sarif"][..],
            "sarif works with --lint and --syntax",
        ),
        (
            &["--output-format", "sarif"],
            "sarif works with --lint and --syntax",
        ),
        (
            &["--stdin", "--output-format", "json"],
            "json works with --check, --lint, --syntax",
        ),
        (&["--rules", "--output-format", "json"], "json works with"),
    ] {
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout(&o), "", "{args:?}");
        assert!(stderr(&o).contains(msg), "{args:?}: {}", stderr(&o));
    }
}

#[cfg(unix)]
#[test]
fn json_and_sarif_escape_hostile_names_and_messages() {
    let d = Dir::new("jsonesc");
    // quotes, a backslash, a control character, a legacy workflow command and non-ASCII,
    // in the file name and in the source text a message quotes
    d.write(
        "q\"\\\x01##[e]é.groovy",
        "def m = ['\"##[x]é\x1b': 1, '\"##[x]é\x1b': 2]\n",
    );
    let o = run(&d.0, &["--lint", "--output-format", "json"], None);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let s = stdout(&o);
    // `[` is `[`
    let lb = "\\u005b";
    let path = format!(r#"{{"path":"q\"\\\u0001##{lb}e]é.groovy","status":"unchanged","#);
    assert!(s.contains(&path), "{s}");
    let message =
        format!("\"message\":\"Key `\\\"##{lb}x]é\\u001b` appears more than once in this map\"");
    assert!(s.contains(&message), "{s}");
    let o = run(&d.0, &["--lint", "--output-format", "sarif"], None);
    let s = stdout(&o);
    assert!(
        s.contains(r#""uri":"q%22%5C%01%23%23%5Be%5D%C3%A9.groovy""#),
        "{s}"
    );
    for out in [
        stdout(&o),
        stdout(&run(&d.0, &["--lint", "--output-format", "json"], None)),
    ] {
        assert!(!out.contains("##["), "{out}");
        assert!(!out.chars().any(|c| c.is_control() && c != '\n'), "{out:?}");
    }
}

const FIXABLE: &str = "import a.Used\nimport a.Unused\nimport a.Used\nimport b.C; import b.D\nimport e.F // why\n\nUsed u = new Used();\nprintln u;\n";
const FIXED: &str =
    "import a.Used\nimport b.C; import b.D\nimport e.F // why\n\nUsed u = new Used()\nprintln u\n";

#[test]
fn lint_fix_removes_imports_and_semicolons_and_reports_the_rest() {
    let d = Dir::new("fix");
    d.write("A.groovy", FIXABLE);
    let l = run(&d.0, &["--lint"], None);
    assert_eq!(l.status.code(), Some(1));
    assert_eq!(
        stdout(&l),
        "A.groovy:2:1: UnusedImport: The [a.Unused] import is never referenced (fixable)\n\
         A.groovy:3:1: DuplicateImport: Duplicate import: import a.Used (fixable)\n\
         A.groovy:4:1: UnusedImport: The [b.C] import is never referenced\n\
         A.groovy:4:13: UnusedImport: The [b.D] import is never referenced\n\
         A.groovy:5:1: UnusedImport: The [e.F] import is never referenced\n\
         A.groovy:7:20: UnnecessarySemicolon: Semicolons as line endings can be removed safely (fixable)\n\
         A.groovy:8:10: UnnecessarySemicolon: Semicolons as line endings can be removed safely (fixable)\n"
    );
    assert_eq!(d.read("A.groovy"), FIXABLE, "plain --lint never writes");

    let f = run(&d.0, &["--lint", "--fix"], None);
    assert_eq!(f.status.code(), Some(1), "violations remain");
    assert_eq!(d.read("A.groovy"), FIXED);
    assert_eq!(
        stdout(&f),
        "A.groovy:2:1: UnusedImport: The [b.C] import is never referenced\n\
         A.groovy:2:13: UnusedImport: The [b.D] import is never referenced\n\
         A.groovy:3:1: UnusedImport: The [e.F] import is never referenced\n"
    );
    assert_eq!(
        stderr(&f),
        "fixed 4 violations in 1 files (DuplicateImport 1, UnnecessarySemicolon 2, UnusedImport 1)\n\
         3 violations in 1 files (UnusedImport 3)\n"
    );
    // a second run changes nothing
    let again = run(&d.0, &["--lint", "--fix"], None);
    assert_eq!(again.status.code(), Some(1));
    assert_eq!(d.read("A.groovy"), FIXED);
    assert!(
        stderr(&again).starts_with("fixed 0 violations in 0 files\n"),
        "{}",
        stderr(&again)
    );

    // nothing left: exit 0; a disabled rule is neither fixed nor reported
    d.write("B.groovy", "x = 1;\n");
    d.write(".groovyfmt.toml", "[lint]\nUnusedImport = false\n");
    let o = run(&d.0, &["--lint", "--fix", "B.groovy", "A.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(d.read("B.groovy"), "x = 1\n");
    assert_eq!(d.read("A.groovy"), FIXED);

    // a file that doesn't parse is never touched
    d.write("Broken.groovy", "import a.B;\ndef x = {\n");
    let o = run(&d.0, &["--lint", "--fix", "Broken.groovy"], None);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(d.read("Broken.groovy"), "import a.B;\ndef x = {\n");

    for args in [
        &["--fix"][..],
        &["--check", "--fix"],
        &["--syntax", "--fix"],
    ] {
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(stderr(&o).contains("--fix needs --lint"), "{args:?}");
    }
}

#[test]
fn lint_fix_json_reports_fixed_files() {
    let d = Dir::new("fixjson");
    d.write("A.groovy", FIXABLE);
    d.write("Clean.groovy", "x = 1\n");
    let o = run(&d.0, &["--lint", "--fix", "--output-format", "json"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stderr(&o), "");
    let s = stdout(&o);
    assert!(
        s.contains(r#"{"path":"A.groovy","status":"fixed","skip_reason":null,"diagnostics":[{"rule":"UnusedImport","severity":"warning","message":"The [b.C] import is never referenced","line":2,"column":1,"end_line":2,"end_column":11,"fixable":false},"#),
        "{s}"
    );
    assert!(
        s.contains(r#"{"path":"Clean.groovy","status":"unchanged","#),
        "{s}"
    );
    assert!(s.ends_with(&(r#""summary":{"files":2,"unchanged":1,"would_reformat":0,"reformatted":0,"fixed":1,"skipped":0,"errors":0,"diagnostics":3}}"#.to_owned() + "\n")), "{s}");
}

#[cfg(unix)]
#[test]
fn lint_fix_writes_like_formatting_does() {
    use std::os::unix::fs::MetadataExt;
    // a link out of the working directory is refused; a link inside is fixed through
    let d = Dir::new("fixlinks");
    let out = Dir::new("fixlinksout");
    let target = out.write("Real.groovy", "x = 1;\n");
    std::os::unix::fs::symlink(&target, d.0.join("x.groovy")).unwrap();
    d.write("real/In.groovy", "y = 2;\n");
    std::os::unix::fs::symlink("real/In.groovy", d.0.join("in.groovy")).unwrap();
    let o = run(&d.0, &["--lint", "--fix", "x.groovy", "in.groovy"], None);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("x.groovy: symlink target is outside the working directory"),
        "{}",
        stderr(&o)
    );
    assert_eq!(out.read("Real.groovy"), "x = 1;\n");
    assert_eq!(d.read("real/In.groovy"), "y = 2\n");
    assert!(
        std::fs::symlink_metadata(d.0.join("in.groovy"))
            .unwrap()
            .is_symlink()
    );
    // excluded files are left alone; hard links stay linked
    d.write(".groovyfmt.toml", "exclude = [\"gen\"]\n");
    d.write("gen/G.groovy", "z = 3;\n");
    let h = d.write("H.groovy", "h = 4;\n");
    std::fs::hard_link(&h, d.0.join("H2.groovy")).unwrap();
    let o = run(&d.0, &["--lint", "--fix", "gen/G.groovy", "H.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(d.read("gen/G.groovy"), "z = 3;\n");
    assert_eq!(d.read("H2.groovy"), "h = 4\n");
    assert_eq!(std::fs::metadata(&h).unwrap().nlink(), 2);
}

const UGLY_PATCH: &str = "--- a/src/A.groovy\n+++ b/src/A.groovy\n@@ -1,5 +1,5 @@\n class A {\n-def f() {\n-println 'x'   \n-}\n+    def f() {\n+        println 'x'\n+    }\n }\n";

#[test]
fn check_diff_prints_a_patch_and_writes_nothing() {
    let d = Dir::new("diff");
    d.write("src/A.groovy", UGLY);
    d.write("B.groovy", PRETTY);
    let o = run(&d.0, &["--check", "--diff"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stdout(&o), UGLY_PATCH);
    assert_eq!(stderr(&o), "would reformat 1 of 2 files\n");
    assert_eq!(d.read("src/A.groovy"), UGLY);
    // no final newline
    d.write("src/A.groovy", "x = 1");
    let o = run(&d.0, &["--check", "--diff", "src"], None);
    assert_eq!(
        stdout(&o),
        "--- a/src/A.groovy\n+++ b/src/A.groovy\n@@ -1,1 +1,1 @@\n-x = 1\n\\ No newline at end of file\n+x = 1\n"
    );
    d.write("src/A.groovy", PRETTY);
    let o = run(&d.0, &["--check", "--diff"], None);
    assert_eq!((o.status.code(), stdout(&o)), (Some(0), String::new()));

    for (args, msg) in [
        (
            &["--diff"][..],
            "--diff works with --check and --lint --fix",
        ),
        (
            &["--lint", "--diff"],
            "--diff works with --check and --lint --fix",
        ),
        (
            &["--check", "--diff", "--output-format", "json"],
            "can't be combined with --output-format",
        ),
        (
            &["--check", "--diff", "--output-format", "github"],
            "can't be combined with --output-format",
        ),
    ] {
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(stderr(&o).contains(msg), "{args:?}: {}", stderr(&o));
    }
}

#[test]
fn lint_fix_diff_previews_the_fix() {
    let d = Dir::new("fixdiff");
    d.write("A.groovy", "import a.B\nx = 1;\n");
    let o = run(&d.0, &["--lint", "--fix", "--diff"], None);
    assert_eq!(o.status.code(), Some(1), "nothing was fixed on disk");
    assert_eq!(
        stdout(&o),
        "--- a/A.groovy\n+++ b/A.groovy\n@@ -1,2 +1,1 @@\n-import a.B\n-x = 1;\n+x = 1\n"
    );
    assert_eq!(
        stderr(&o),
        "would fix 2 violations in 1 files (UnnecessarySemicolon 1, UnusedImport 1)\n0 violations in 1 files\n"
    );
    assert_eq!(d.read("A.groovy"), "import a.B\nx = 1;\n");
}

#[cfg(unix)]
#[test]
fn diffs_never_print_control_characters_or_workflow_commands() {
    let d = Dir::new("diffesc");
    // a changed line holding `##[` or ESC, a context line starting `::`, a name with a quote
    d.write("a/A.groovy", "class A {\ndef s = '##[error]x'\n}\n");
    d.write("b/B.groovy", "class B {\ndef s = '\x1b[31m'\n}\n");
    d.write(
        "c/C.groovy",
        "class C {\n  def f() {\n    def s = '''\n::warning::x\n'''\n  g()\n  }\n}\n",
    );
    d.write("d/q\"x.groovy", UGLY);
    d.write("e/Ok.groovy", UGLY);
    let o = run(&d.0, &["--check", "--diff"], None);
    assert_eq!(o.status.code(), Some(1));
    let out = stdout(&o);
    for name in ["a/A.groovy", "b/B.groovy", "c/C.groovy", "d/q\"x.groovy"] {
        assert!(
            out.contains(&format!("# groovyfmt: {name}: diff not shown, as the file name or a changed line holds a control character or a CI workflow command\n")),
            "{name}: {out}"
        );
    }
    assert!(out.contains("--- a/e/Ok.groovy\n"), "{out}");
    // names that would split or forge the `---` / `+++` header lines
    for name in [
        "f/x\n+++ b/other.groovy",
        "f/a\rb.groovy",
        "f/a\tb.groovy",
        "f/x##[e].groovy",
    ] {
        d.write(name, UGLY);
    }
    let o = run(&d.0, &["--check", "--diff", "f"], None);
    assert_eq!(o.status.code(), Some(1));
    let out = stdout(&o);
    assert_eq!(out.matches("# groovyfmt: f/").count(), 4, "{out}");
    assert!(
        out.lines().all(|l| l.starts_with("# groovyfmt: ")),
        "{out:?}"
    );
    assert!(!out.contains("##["), "{out}");
    // CR LF content diffs as it is
    d.write("g/W.groovy", "class W {\r\ndef f() {}\r\n}\r\n");
    let o = run(&d.0, &["--check", "--diff", "g"], None);
    assert_eq!(
        stdout(&o),
        "--- a/g/W.groovy\n+++ b/g/W.groovy\n@@ -1,3 +1,3 @@\n class W {\r\n-def f() {}\r\n+    def f() {}\r\n }\r\n"
    );
    assert!(!out.contains("##["), "{out}");
    assert!(!out.chars().any(|c| c.is_control() && c != '\n'), "{out:?}");
    assert!(
        out.lines().all(|l| !l.trim_start().starts_with("::")),
        "{out}"
    );
}

#[test]
fn lines_limits_formatting_to_ranges() {
    let d = Dir::new("lines");
    let src = "class A {\ndef f() {\nprintln 1   \nprintln 2\n}\n}\n";
    d.write("A.groovy", src);
    let o = run(&d.0, &["--check", "--lines", "3-3", "A.groovy"], None);
    assert_eq!(
        (o.status.code(), stdout(&o)),
        (Some(1), "A.groovy\n".into())
    );
    let o = run(&d.0, &["--lines", "3-3", "--lines=5-5", "A.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let want = "class A {\ndef f() {\n        println 1\nprintln 2\n    }\n}\n";
    assert_eq!(d.read("A.groovy"), want);
    assert_eq!(
        run(
            &d.0,
            &["--check", "--lines", "3-3", "--lines", "5-5", "A.groovy"],
            None
        )
        .status
        .code(),
        Some(0),
        "stable"
    );
    let o = run(&d.0, &["--stdin", "--lines", "4-4"], Some(src));
    assert_eq!(
        stdout(&o),
        "class A {\ndef f() {\nprintln 1   \n        println 2\n}\n}\n"
    );
    // lines outside the file change nothing
    let o = run(&d.0, &["--stdin", "--lines", "40-50"], Some(src));
    assert_eq!(stdout(&o), src);

    d.write("B.groovy", src);
    for (args, msg) in [
        (
            &["--lines", "3-3"][..],
            "--lines works on one file, or with --stdin",
        ),
        (
            &["--lines", "3-3", "A.groovy", "B.groovy"],
            "--lines works on one file",
        ),
        (&["--lines", "3-3", "."], "--lines works on one file"),
        (
            &["--lint", "--lines", "3-3", "A.groovy"],
            "--lines works on one file",
        ),
        (&["--lines", "3", "A.groovy"], "--lines takes START-END"),
        (&["--lines", "0-2", "A.groovy"], "--lines takes START-END"),
        (&["--lines", "5-2", "A.groovy"], "--lines takes START-END"),
    ] {
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(stderr(&o).contains(msg), "{args:?}: {}", stderr(&o));
    }
    assert_eq!(d.read("B.groovy"), src);
}

#[test]
fn changed_since_limits_the_run_to_files_git_sees_as_changed() {
    let d = Dir::new("changed");
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args(args)
            .current_dir(&d.0)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .output()
            .expect("git");
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    // outside a repository: a clear error
    let o = run(&d.0, &["--check", "--changed-since", "HEAD"], None);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("--changed-since: git diff: "),
        "{}",
        stderr(&o)
    );

    git(&["init", "-q"]);
    d.write("Same.groovy", UGLY);
    d.write("Edited.groovy", UGLY);
    d.write("sub/Deleted.groovy", UGLY);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "x"]);
    d.write("Edited.groovy", &format!("{UGLY}// more\n"));
    d.write("sub/New.groovy", UGLY);
    d.write(".gitignore", "Ignored.groovy\n");
    d.write("Ignored.groovy", UGLY);
    std::fs::remove_file(d.0.join("sub/Deleted.groovy")).unwrap();
    let o = run(&d.0, &["--check", "--changed-since", "HEAD"], None);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert_eq!(stdout(&o), "Edited.groovy\nsub/New.groovy\n");
    // from a subdirectory, only what is under it
    let o = run(&d.0.join("sub"), &["--check", "--changed-since=HEAD"], None);
    assert_eq!(stdout(&o), "New.groovy\n");
    // whole files are formatted
    let o = run(&d.0, &["--changed-since", "HEAD"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(d.read("Edited.groovy"), format!("{PRETTY}// more\n"));
    assert_eq!(d.read("Same.groovy"), UGLY);

    for (rev, msg) in [
        (
            "--output=x",
            "--changed-since needs a git revision, not `--output=x`",
        ),
        ("", "--changed-since needs a git revision"),
        ("no-such-rev", "--changed-since: git diff: "),
    ] {
        let o = run(&d.0, &["--check", "--changed-since", rev], None);
        assert_eq!(o.status.code(), Some(2), "{rev}");
        assert!(stderr(&o).contains(msg), "{rev}: {}", stderr(&o));
    }
    assert!(!d.0.join("x").exists());
}

#[test]
fn explain_describes_one_rule() {
    let d = Dir::new("explain");
    let o = run(&d.0, &["--explain", "UnnecessarySemicolon"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "UnnecessarySemicolon: `;` at the end of a line\n\n\
         On by default: yes. Here: on. `groovyfmt --lint --fix` fixes it.\n\n\
         Why: Groovy ends a statement at the line break, so a `;` there is Java habit and clutter.\n\n\
         Flagged:\n    def name = 'x';\n    println name;\n\n\
         Instead:\n    def name = 'x'\n    println name\n\n\
         Turn it off in .groovyfmt.toml:\n    [lint]\n    UnnecessarySemicolon = false\n"
    );
    // any case; the config decides "Here"
    d.write(".groovyfmt.toml", "[lint]\nUnnecessaryGString = true\n");
    let o = run(&d.0, &["--explain=unnecessarygstring"], None);
    assert!(
        stdout(&o).contains("On by default: no. Here: on. Not fixed automatically.")
            && stdout(&o).ends_with(
                "Turn it off in .groovyfmt.toml:\n    [lint]\n    UnnecessaryGString = false\n"
            ),
        "{}",
        stdout(&o)
    );
    let o = run(&d.0, &["--explain", "Nope"], None);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("unknown rule `Nope` (known: UnusedImport, "),
        "{}",
        stderr(&o)
    );
    // --rules keeps its format
    let o = run(&d.0, &["--rules"], None);
    assert!(
        stdout(&o).starts_with("on   UnusedImport           import whose name"),
        "{}",
        stdout(&o)
    );
    assert_eq!(stdout(&o).lines().count(), groovyfmt::lint::RULES.len());
}

#[test]
fn option_parsing_edge_cases() {
    let d = Dir::new("args");
    d.write("A.groovy", UGLY);
    d.write("-x.groovy", UGLY);
    // a boolean flag takes no value, so `--check=false` can't turn check on
    for bad in ["--check=false", "--lint=true", "--strict=no", "-v=1"] {
        let o = run(&d.0, &[bad, "A.groovy"], None);
        assert_eq!(o.status.code(), Some(2), "{bad}");
        assert_eq!(d.read("A.groovy"), UGLY, "{bad}");
    }
    assert!(stderr(&run(&d.0, &["--check=false"], None)).contains("--check doesn't take a value"));
    // `--` ends the options
    let o = run(&d.0, &["--check", "--", "-x.groovy"], None);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert_eq!(stdout(&o), "-x.groovy\n");
    assert_eq!(
        run(&d.0, &["--", "--check"], None).status.code(),
        Some(2),
        "a path"
    );
    // paths aren't silently ignored with --stdin
    let o = run(&d.0, &["--stdin", "A.groovy"], Some(UGLY));
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--stdin-filepath"), "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
}

#[test]
fn non_utf8_file_is_skipped_not_fatal() {
    let d = Dir::new("latin1");
    std::fs::write(d.0.join("Latin.groovy"), b"def s = 'caf\xe9'\n").unwrap();
    d.write("Ok.groovy", UGLY);
    let w = run(&d.0, &["-v"], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert!(
        stderr(&w).contains("Latin.groovy: not valid UTF-8"),
        "{}",
        stderr(&w)
    );
    assert_eq!(d.read("Ok.groovy"), PRETTY, "other files still formatted");
    assert_eq!(run(&d.0, &["--lint"], None).status.code(), Some(0));
}

#[test]
fn stdin_mode() {
    let d = Dir::new("stdin");
    let o = run(&d.0, &["--stdin"], Some(UGLY));
    assert!(o.status.success());
    assert_eq!(stdout(&o), PRETTY);
    let same = run(&d.0, &["--stdin"], Some(PRETTY));
    assert_eq!(stdout(&same), PRETTY);
    let bad = run(&d.0, &["--stdin"], Some("def x = {\n"));
    assert_eq!(bad.status.code(), Some(2));
    assert_eq!(stdout(&bad), "", "editors must not get partial output");
}

#[test]
fn stdin_filepath_finds_config_from_its_directory() {
    let d = Dir::new("stdinpath");
    d.write("proj/.groovyfmt.toml", "indent = 2\n");
    let src = "a {\nb()\n}\n";
    // cwd has no config: the default indent
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some(src))),
        "a {\n    b()\n}\n"
    );
    // the file need not exist, nor its directory
    for path in ["proj/src/deep/New.groovy", "proj/x/../Gone.groovy"] {
        let o = run(&d.0, &["--stdin", "--stdin-filepath", path], Some(src));
        assert_eq!(o.status.code(), Some(0), "{path}: {}", stderr(&o));
        assert_eq!(stdout(&o), "a {\n  b()\n}\n", "{path}");
    }
    let abs = d.0.join("proj/A.groovy");
    let o = run(
        &d.0,
        &["--stdin", &format!("--stdin-filepath={}", abs.display())],
        Some(src),
    );
    assert_eq!(stdout(&o), "a {\n  b()\n}\n");

    let alone = run(&d.0, &["--stdin-filepath", "proj/A.groovy"], None);
    assert_eq!(alone.status.code(), Some(2));
    assert!(stderr(&alone).contains("--stdin-filepath needs --stdin"));
    let check = run(&d.0, &["--check", "--stdin-filepath", "A.groovy"], None);
    assert_eq!(check.status.code(), Some(2));
}

#[test]
fn stdin_filepath_excluded_echoes_input() {
    let d = Dir::new("stdinexcl");
    d.write(".groovyfmt.toml", "exclude = [\"gen/**\"]\n");
    let o = run(
        &d.0,
        &["--stdin", "--stdin-filepath", "gen/A.groovy"],
        Some(UGLY),
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stdout(&o), UGLY);
    // even input that couldn't be formatted
    let broken = run(
        &d.0,
        &["--stdin", "--stdin-filepath", "gen/B.groovy"],
        Some("def x = {\n"),
    );
    assert_eq!(broken.status.code(), Some(0));
    assert_eq!(stdout(&broken), "def x = {\n");
    let kept = run(
        &d.0,
        &["--stdin", "--stdin-filepath", "src/A.groovy"],
        Some(UGLY),
    );
    assert_eq!(stdout(&kept), PRETTY);
}

#[test]
fn stdin_filepath_names_the_file_in_errors() {
    let d = Dir::new("stdinname");
    let args = ["--stdin", "--stdin-filepath", "src/::x,y.groovy"];
    let t = run(&d.0, &args, Some("def x = {\n"));
    assert_eq!(t.status.code(), Some(2));
    assert!(
        stderr(&t).contains("src/::x,y.groovy: not formatted: "),
        "{}",
        stderr(&t)
    );
    let mut gh = args.to_vec();
    gh.extend(["--output-format", "github"]);
    let g = run(&d.0, &gh, Some("def x = {\n"));
    assert_eq!(g.status.code(), Some(2));
    assert_eq!(stdout(&g), "");
    assert!(
        stderr(&g).starts_with("::error file=src/%3A%3Ax%2Cy.groovy::Not formatted: "),
        "{}",
        stderr(&g)
    );
    let colon = run(
        &d.0,
        &[
            "--stdin",
            "--stdin-filepath",
            "::warning::x.groovy",
            "--output-format",
            "github",
        ],
        Some("def x = {\n"),
    );
    assert!(
        stderr(&colon).starts_with("::error file=./%3A%3Awarning%3A%3Ax.groovy::"),
        "{}",
        stderr(&colon)
    );
}

#[test]
fn exclude_prunes_walk_and_explicit_files() {
    let d = Dir::new("exclude");
    d.write(
        "proj/.groovyfmt.toml",
        "exclude = [\"gen\", \"**/*.gradle\", \"src/*/Skip.groovy\"]\n",
    );
    d.write("proj/gen/G.groovy", UGLY);
    d.write("proj/a/b/build.gradle", UGLY);
    d.write("proj/src/x/Skip.groovy", UGLY);
    d.write("proj/src/x/y/Skip.groovy", UGLY);
    d.write("proj/src/A.groovy", UGLY);
    d.write("proj/other/gen/O.groovy", UGLY);
    let proj = d.0.join("proj");
    let c = run(&proj, &["--check"], None);
    assert_eq!(
        stdout(&c),
        "other/gen/O.groovy\nsrc/A.groovy\nsrc/x/y/Skip.groovy\n",
        "gen pruned at the config dir only; * stays in one segment; ** spans any depth"
    );
    // pre-commit passes files explicitly
    let explicit = run(
        &proj,
        &[
            "--check",
            "gen/G.groovy",
            "a/b/build.gradle",
            "./src/x/Skip.groovy",
            "src/A.groovy",
        ],
        None,
    );
    assert_eq!(stdout(&explicit), "src/A.groovy\n");
    // patterns are relative to the config file, not the working directory
    let from_sub = run(
        &proj.join("src"),
        &["--check", "../gen/G.groovy", "."],
        None,
    );
    assert_eq!(stdout(&from_sub), "A.groovy\nx/y/Skip.groovy\n");
    let named_dir = run(&proj, &["--check", "gen"], None);
    assert_eq!(named_dir.status.code(), Some(0));
    assert_eq!(stdout(&named_dir), "");

    d.write("proj/.groovyfmt.toml", "exclude = \"gen\"\n");
    let bad = run(&proj, &["--check"], None);
    assert_eq!(bad.status.code(), Some(2));
    assert!(
        stderr(&bad).contains("`exclude` must be a list of glob strings"),
        "{}",
        stderr(&bad)
    );
}

#[test]
fn lint_text_github_and_config() {
    let d = Dir::new("lint");
    d.write("A.groovy", "import a.b.Unused\ndef s = \"plain\"\n");
    let t = run(&d.0, &["--lint"], None);
    assert_eq!(t.status.code(), Some(1));
    assert!(
        stdout(&t).contains("A.groovy:1:1: UnusedImport:"),
        "{}",
        stdout(&t)
    );
    assert!(!stdout(&t).contains("UnnecessaryGString"), "off by default");

    let g = run(&d.0, &["--lint", "--output-format", "github"], None);
    assert!(
        stdout(&g).starts_with("::warning file=") && stdout(&g).contains("title=UnusedImport::"),
        "{}",
        stdout(&g)
    );

    d.write(
        ".groovyfmt.toml",
        "[lint]\nUnusedImport = false\nUnnecessaryGString = true\n",
    );
    let c = run(&d.0, &["--lint"], None);
    assert!(
        !stdout(&c).contains("UnusedImport") && stdout(&c).contains("UnnecessaryGString"),
        "{}",
        stdout(&c)
    );

    d.write(".groovyfmt.toml", "[lint]\nUnusedImport = false\n");
    assert_eq!(run(&d.0, &["--lint"], None).status.code(), Some(0));
}

#[test]
fn indent_from_config_and_flag() {
    let d = Dir::new("indent");
    d.write(".groovyfmt.toml", "indent = 2\n");
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some("a {\nb()\n}\n"))),
        "a {\n  b()\n}\n"
    );
    assert_eq!(
        stdout(&run(
            &d.0,
            &["--stdin", "--indent=8"],
            Some("a {\nb()\n}\n")
        )),
        "a {\n        b()\n}\n"
    );
}

#[test]
fn use_tabs_from_config() {
    let d = Dir::new("tabs");
    let tabbed = include_str!("fixtures/tabs_build.gradle");
    d.write("build.gradle", tabbed);
    d.write("A.groovy", UGLY);
    let spaces = run(&d.0, &["--check"], None);
    assert_eq!(stdout(&spaces), "A.groovy\nbuild.gradle\n");
    d.write(".groovyfmt.toml", "use_tabs = true\n");
    let c = run(&d.0, &["--check"], None);
    assert_eq!(c.status.code(), Some(1), "{}", stderr(&c));
    assert_eq!(stdout(&c), "A.groovy\n", "the tab-indented fixture fits");
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some(UGLY))),
        "class A {\n\tdef f() {\n\t\tprintln 'x'\n\t}\n}\n"
    );
    d.write(
        ".groovyfmt.toml",
        "use_tabs = true\ncontinuation_indent = 8\n",
    );
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some("f(a,\nb)\n"))),
        "f(a,\n\t\tb)\n",
        "two tab widths"
    );
}

#[test]
fn suggest_style_picks_the_style_that_changes_fewest_files() {
    let d = Dir::new("suggest");
    let cont8 = "def f() {\n    render(view: 'x',\n            model: 1)\n}\n";
    let tabbed = include_str!("fixtures/tabs_build.gradle");
    // two files fit the expected style and the third doesn't
    let cases: &[(&str, &[&str], &str)] = &[
        // TWO also fits indent 2 with continuation 4: a tie goes to the simpler style
        ("two", &[TWO, TWO, PRETTY], "indent = 2\n"),
        ("tabs", &[tabbed, tabbed, PRETTY], "use_tabs = true\n"),
        (
            "cont8",
            &[cont8, cont8, "f(a,\n    b)\n"],
            "continuation_indent = 8\n",
        ),
    ];
    for (name, srcs, want) in cases {
        for (i, s) in srcs.iter().enumerate() {
            d.write(&format!("{name}/F{i}.groovy"), s);
        }
        let o = run(&d.0, &["--suggest-style", name], None);
        assert_eq!(o.status.code(), Some(0), "{name}: {}", stderr(&o));
        assert_eq!(
            stdout(&o),
            format!("# suggested by groovyfmt --suggest-style: 2 of 3 files unchanged\n{want}"),
            "{name}: {}",
            stderr(&o)
        );
        assert!(
            stderr(&o).contains(&format!("* {}", want.trim_end())),
            "{}",
            stderr(&o)
        );
        for (i, s) in srcs.iter().enumerate() {
            assert_eq!(d.read(&format!("{name}/F{i}.groovy")), *s, "never writes");
        }
    }

    // defaults, and a tie where every style fits goes to the defaults
    d.write("fits/A.groovy", PRETTY);
    d.write("flat/A.groovy", "x = 1\n");
    for dir in ["fits", "flat"] {
        let o = run(&d.0, &["--suggest-style", dir], None);
        assert_eq!(o.status.code(), Some(0), "{dir}: {}", stderr(&o));
        assert_eq!(
            stdout(&o),
            "# the defaults fit best: 1 of 1 files unchanged\n",
            "{dir}"
        );
    }
    // every style changes the file: fewer lines changed decides, not candidate order
    d.write(
        "allchange/A.groovy",
        "class A {\n\tdef f() {\n\t  x(1,\n\t\t\t2)\n\t}\n}\n",
    );
    let o = run(&d.0, &["--suggest-style", "allchange"], None);
    assert_eq!(
        stdout(&o),
        "# suggested by groovyfmt --suggest-style: 0 of 1 files unchanged\nuse_tabs = true\n",
        "{}",
        stderr(&o)
    );
    // a missing final newline is a change, though no line's text differs
    d.write("eof/A.groovy", "x = 1");
    let o = run(&d.0, &["--suggest-style", "eof"], None);
    assert_eq!(
        stdout(&o),
        "# the defaults fit best: 0 of 1 files unchanged\n"
    );

    // the config's style keys don't matter, its other settings do
    d.write("cfg/.groovyfmt.toml", "indent = 8\nexclude = [\"gen\"]\n");
    d.write("cfg/A.groovy", TWO);
    d.write("cfg/gen/B.groovy", PRETTY);
    d.write("cfg/gen/C.groovy", PRETTY);
    let o = run(&d.0, &["--suggest-style", "cfg"], None);
    assert_eq!(
        stdout(&o),
        "# suggested by groovyfmt --suggest-style: 1 of 1 files unchanged\nindent = 2\n",
        "{}",
        stderr(&o)
    );
}

#[test]
fn suggest_style_without_anything_to_measure() {
    let d = Dir::new("suggestnone");
    d.write("empty/notes.txt", "");
    d.write("broken/A.groovy", "def x = {\n");
    for (dir, msg) in [
        ("empty", "no Groovy files found, nothing to suggest"),
        (
            "broken",
            "none of the 1 files could be formatted, nothing to suggest",
        ),
    ] {
        let o = run(&d.0, &["--suggest-style", dir], None);
        assert_eq!(o.status.code(), Some(2), "{dir}: {}", stderr(&o));
        assert_eq!(stdout(&o), "", "{dir}");
        assert!(stderr(&o).contains(msg), "{dir}: {}", stderr(&o));
    }
    for (args, msg) in [
        (&["--suggest-style", "--check"][..], "can't be combined"),
        (&["--lint", "--suggest-style"], "can't be combined"),
        (&["--suggest-style", "--stats"], "can't be combined"),
        (
            &["--suggest-style", "--indent", "2"],
            "--indent can't be combined with --suggest-style",
        ),
    ] {
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(stderr(&o).contains(msg), "{args:?}: {}", stderr(&o));
    }
}

#[test]
fn indent_after_labels_from_config() {
    let d = Dir::new("labels");
    let spec = "def 'x'() {\nwhen:\nf()\nthen:\nok()\n}\n";
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some(spec))),
        "def 'x'() {\n    when:\n    f()\n    then:\n    ok()\n}\n"
    );
    d.write(".groovyfmt.toml", "indent_after_labels = true\n");
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some(spec))),
        "def 'x'() {\n    when:\n        f()\n    then:\n        ok()\n}\n"
    );
}

#[test]
fn normalize_spacing_from_config() {
    let d = Dir::new("spacing");
    let src = "if(x){\nfoo (a,b)\n}\n";
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some(src))),
        "if(x){\n    foo (a,b)\n}\n",
        "off by default"
    );
    d.write(".groovyfmt.toml", "normalize_spacing = true\n");
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some(src))),
        "if (x) {\n    foo(a, b)\n}\n"
    );
    d.write(".groovyfmt.toml", "normalize_spacing = 1\n");
    let o = run(&d.0, &["--stdin"], Some(src));
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("`normalize_spacing` must be true or false"));
}

#[test]
fn normalize_spacing_with_lines_diff_fix_and_json() {
    let d = Dir::new("spacingmodes");
    d.write(".groovyfmt.toml", "normalize_spacing = true\n");
    let src = "class A {\ndef f() {\nfoo(a,b)\nbar(c,d)\n}\n}\n";

    // --lines: only the range changes, spacing included, and a re-run changes nothing
    d.write("L.groovy", src);
    let o = run(&d.0, &["--lines", "3-3", "L.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let want = "class A {\ndef f() {\n        foo(a, b)\nbar(c,d)\n}\n}\n";
    assert_eq!(d.read("L.groovy"), want);
    let o = run(&d.0, &["--check", "--lines", "3-3", "L.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "stable: {}", stdout(&o));

    // --check --diff: the patch applies to give the formatted file
    d.write("D.groovy", src);
    let o = run(&d.0, &["--check", "--diff", "D.groovy"], None);
    assert_eq!(o.status.code(), Some(1));
    d.write("D.patch", &stdout(&o));
    let formatted = stdout(&run(&d.0, &["--stdin"], Some(src)));
    assert!(formatted.contains("foo(a, b)"), "{formatted}");
    let applied = Command::new("git")
        .args(["apply", "D.patch"])
        .current_dir(&d.0)
        .output()
        .unwrap();
    assert!(applied.status.success(), "{:?}", applied);
    assert_eq!(d.read("D.groovy"), formatted);

    // --lint --fix, then formatting: the fix's checks pass on a file that also gets spacing
    // changes, and formatting it twice changes nothing the second time
    d.write("F.groovy", "import a.B\ndef x=foo(a,b);\n");
    let o = run(&d.0, &["--lint", "--fix", "F.groovy"], None);
    assert_eq!(d.read("F.groovy"), "def x=foo(a,b)\n", "{}", stderr(&o));
    assert_eq!(run(&d.0, &["F.groovy"], None).status.code(), Some(0));
    assert_eq!(d.read("F.groovy"), "def x = foo(a, b)\n");
    let o = run(&d.0, &["--check", "F.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "idempotent");

    // a spacing-only change is a file to reformat in JSON
    d.write("J.groovy", "foo(a,b)\n");
    let o = run(
        &d.0,
        &["--check", "--output-format", "json", "J.groovy"],
        None,
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).contains(&json_file("J.groovy", "would_reformat")),
        "{}",
        stdout(&o)
    );
}

#[test]
fn max_line_length_from_config_with_lines_diff_and_suggest_style() {
    let d = Dir::new("wrap");
    let src = "class A {\ndef f() {\nfoo(alpha, beta, gamma, delta, epsilon)\nbar(alpha, beta, gamma, delta, epsilon)\n}\n}\n";
    let plain = "class A {\n    def f() {\n        foo(alpha, beta, gamma, delta, epsilon)\n        bar(alpha, beta, gamma, delta, epsilon)\n    }\n}\n";
    assert_eq!(
        stdout(&run(&d.0, &["--stdin"], Some(src))),
        plain,
        "off by default"
    );
    d.write(".groovyfmt.toml", "max_line_length = 40\n");
    let wrapped = "class A {\n    def f() {\n        foo(\n            alpha,\n            beta,\n            gamma,\n            delta,\n            epsilon\n        )\n        bar(\n            alpha,\n            beta,\n            gamma,\n            delta,\n            epsilon\n        )\n    }\n}\n";
    assert_eq!(stdout(&run(&d.0, &["--stdin"], Some(src))), wrapped);

    // --check --diff: the patch applies to give the formatted file
    d.write("D.groovy", src);
    let o = run(&d.0, &["--check", "--diff", "D.groovy"], None);
    assert_eq!(o.status.code(), Some(1));
    d.write("D.patch", &stdout(&o));
    let applied = Command::new("git")
        .args(["apply", "D.patch"])
        .current_dir(&d.0)
        .output()
        .unwrap();
    assert!(applied.status.success(), "{:?}", applied);
    assert_eq!(d.read("D.groovy"), wrapped);
    let o = run(&d.0, &["--check", "D.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "idempotent");

    // --lines: only the line in the range is re-indented and wrapped; the lines outside it
    // stay byte for byte
    d.write("L.groovy", src);
    let o = run(&d.0, &["--lines", "4-4", "L.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        d.read("L.groovy"),
        "class A {\ndef f() {\nfoo(alpha, beta, gamma, delta, epsilon)\n        bar(\n            alpha,\n            beta,\n            gamma,\n            delta,\n            epsilon\n        )\n}\n}\n"
    );

    // --suggest-style still runs, with the other settings in the config applied
    let o = run(&d.0, &["--suggest-style", "D.groovy"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));

    for bad in ["39", "401", "\"80\""] {
        d.write(".groovyfmt.toml", &format!("max_line_length = {bad}\n"));
        let o = run(&d.0, &["--stdin"], Some(src));
        assert_eq!(o.status.code(), Some(2));
        assert!(
            stderr(&o).contains("`max_line_length` must be an integer from 40 to 400"),
            "{}",
            stderr(&o)
        );
    }
}

#[test]
fn errors_exit_2_with_specific_messages() {
    // each case in its own directory, so one failure mode can't mask another
    let cases: &[(&str, Option<&str>, &[&str], &str)] = &[
        (
            "cfg",
            Some("indnet = 2\n"),
            &["--check"],
            "unknown key `indnet`",
        ),
        ("bogus", None, &["--bogus"], "unknown option `--bogus`"),
        ("combo", None, &["--check", "--lint"], "can't be combined"),
        (
            "nocfg",
            None,
            &["--config", "nope.toml", "--check"],
            "nope.toml",
        ),
        (
            "missing",
            None,
            &["--check", "missing.groovy"],
            "missing.groovy: no such file or directory",
        ),
        (
            "indent",
            None,
            &["--indent", "0", "--check"],
            "--indent must be 1 to 16",
        ),
        (
            "format",
            None,
            &["--output-format", "xml", "--lint"],
            "unknown --output-format `xml`",
        ),
    ];
    for (name, toml, args, msg) in cases {
        let d = Dir::new(&format!("err-{name}"));
        d.write("A.groovy", PRETTY);
        if let Some(t) = toml {
            d.write(".groovyfmt.toml", t);
        }
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{name}: {}", stderr(&o));
        assert!(
            stderr(&o).contains(msg),
            "{name}: expected `{msg}` in {}",
            stderr(&o)
        );
    }
}

#[test]
fn check_github_output_is_exact() {
    let d = Dir::new("ghcheck");
    d.write("src/A.groovy", UGLY);
    let o = run(&d.0, &["--check", "--output-format", "github"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        "::error file=src/A.groovy::File is not formatted. Run groovyfmt to fix.\n"
    );
}

#[test]
fn lint_github_output_is_exact() {
    let d = Dir::new("ghlint");
    d.write("src/A.groovy", "import a.b.Unused\ndef a = 1;\n");
    let o = run(&d.0, &["--lint", "--output-format", "github"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        "::warning file=src/A.groovy,line=1,col=1,title=UnusedImport::The [a.b.Unused] import is never referenced\n\
         ::warning file=src/A.groovy,line=2,col=10,title=UnnecessarySemicolon::Semicolons as line endings can be removed safely\n"
    );
}

#[test]
fn lint_github_escapes_percent_in_messages() {
    let d = Dir::new("ghlintmsg");
    d.write("A.groovy", "def m = ['a%0Ab': 1, 'a%0Ab': 2]\n");
    let o = run(&d.0, &["--lint", "--output-format", "github"], None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        "::warning file=A.groovy,line=1,col=22,title=DuplicateMapKey::Key `a%250Ab` appears more than once in this map\n"
    );
}

#[cfg(unix)]
#[test]
fn github_output_escapes_the_file_property() {
    let d = Dir::new("ghfile");
    d.write("a,b%2C.groovy", "def a = 1;\n");
    let l = run(&d.0, &["--lint", "--output-format", "github"], None);
    assert_eq!(
        stdout(&l),
        "::warning file=a%2Cb%252C.groovy,line=1,col=10,title=UnnecessarySemicolon::Semicolons as line endings can be removed safely\n"
    );
    d.write("a,b%2C.groovy", UGLY);
    let c = run(&d.0, &["--check", "--output-format", "github"], None);
    assert_eq!(c.status.code(), Some(1));
    assert_eq!(
        stdout(&c),
        "::error file=a%2Cb%252C.groovy::File is not formatted. Run groovyfmt to fix.\n"
    );
}

#[test]
fn files_over_10_mb_are_skipped_unread() {
    let d = Dir::new("big");
    let p = d.write("Big.groovy", UGLY);
    // sparse: no 10 MB written to disk
    std::fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_len(10 * 1024 * 1024 + 1)
        .unwrap();
    let o = run(&d.0, &["-v"], None);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("skipped Big.groovy: larger than 10 MB\n"),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::metadata(&p).unwrap().len(), 10 * 1024 * 1024 + 1);
}

#[cfg(unix)]
#[test]
fn fatal_errors_escape_control_characters_in_paths() {
    let d = Dir::new("diectl");
    let o = run(&d.0, &["--check", "a\x1b[31mb\nc.groovy"], None);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(
        stderr(&o),
        "groovyfmt: a\\u{1b}[31mb\\nc.groovy: no such file or directory\n"
    );
}

#[test]
fn file_name_cannot_start_a_report_line_with_colons() {
    let d = Dir::new("colons");
    d.write("::warning title=x::y.groovy", "def a = 1;\n");
    let o = run(&d.0, &["--lint"], None);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).starts_with("./::warning title=x::y.groovy:1:10: UnnecessarySemicolon"),
        "{}",
        stdout(&o)
    );
    assert!(stdout(&o).lines().all(|l| !l.starts_with("::")));
}

#[test]
fn no_output_line_carries_a_legacy_workflow_command() {
    // the Actions runner takes `##[command]` anywhere in a line, from file names or quoted source
    let d = Dir::new("hashcmd");
    d.write("lint/x##[error]boo.groovy", "def a = 1;\n");
    d.write(
        "lint/M.groovy",
        "def m = ['##[stop-commands]tok': 1, '##[stop-commands]tok': 2]\n",
    );
    d.write("check/y##[add-mask]secret.groovy", UGLY);
    let runs: &[(&[&str], &str, i32)] = &[
        (&["--lint"], "lint", 1),
        (&["--lint", "--output-format", "github"], "lint", 1),
        (&["--check"], "check", 1),
        (&["--check", "--output-format", "github"], "check", 1),
        (&["--check", "nope##[error]x.groovy"], "check", 2),
        (
            &["--stdin", "--stdin-filepath", "z##[error]q.groovy"],
            "check",
            2,
        ),
        (
            &[
                "--stdin",
                "--stdin-filepath",
                "z##[error]q.groovy",
                "--output-format",
                "github",
            ],
            "check",
            2,
        ),
    ];
    for (args, sub, code) in runs {
        // only --stdin reads it; a run that exits first would close the pipe on us
        let input = args.contains(&"--stdin").then_some("def x = {\n");
        let o = run(&d.0.join(sub), args, input);
        assert_eq!(o.status.code(), Some(*code), "{args:?}: {}", stderr(&o));
        let all = stdout(&o) + &stderr(&o);
        assert!(all.contains("##\\["), "{args:?}: {all}");
        assert!(all.lines().all(|l| !l.contains("##[")), "{args:?}: {all}");
    }
}

#[test]
fn a_file_named_several_ways_is_handled_once() {
    // workers writing one file at once used to collide on its temp name and exit 2
    let d = Dir::new("overlap");
    d.write("sub/keep.txt", "");
    let names: Vec<String> = (0..40).map(|i| format!("F{i:02}.groovy")).collect();
    let mut args: Vec<String> = vec![".".into()];
    for n in &names {
        d.write(n, UGLY);
        args.extend([
            n.clone(),
            n.clone(),
            format!("./{n}"),
            format!("sub/../{n}"),
        ]);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut check = vec!["--check"];
    check.extend(&args);
    let c = run(&d.0, &check, None);
    assert_eq!(c.status.code(), Some(1), "{}", stderr(&c));
    assert_eq!(stdout(&c), names.join("\n") + "\n", "each file listed once");
    for _ in 0..5 {
        for n in &names {
            d.write(n, UGLY);
        }
        let w = run(&d.0, &args, None);
        assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
        assert!(
            stderr(&w).starts_with("reformatted 40 of 40 files"),
            "{}",
            stderr(&w)
        );
        for n in &names {
            assert_eq!(d.read(n), PRETTY);
        }
        let leftovers: Vec<_> = std::fs::read_dir(&d.0)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".groovyfmt-"))
            .collect();
        assert!(leftovers.is_empty(), "no temp files left behind");
    }
}

#[test]
fn discovers_groovy_files_and_skips_build_dirs() {
    let d = Dir::new("discover");
    d.write("Jenkinsfile", UGLY);
    d.write("scripts/x.gvy", UGLY);
    d.write("out/O.groovy", UGLY);
    d.write("target/T.groovy", UGLY);
    d.write("README.md", "not groovy");
    let o = run(&d.0, &["--check"], None);
    assert_eq!(stdout(&o), "Jenkinsfile\nscripts/x.gvy\n");
}

#[cfg(unix)]
#[test]
fn symlinks_and_special_files_are_not_followed() {
    let d = Dir::new("links");
    d.write("A.groovy", UGLY);
    std::os::unix::fs::symlink(".", d.0.join("loop1")).unwrap();
    std::os::unix::fs::symlink(".", d.0.join("loop2")).unwrap();
    std::os::unix::fs::symlink("/dev/zero", d.0.join("Zero.groovy")).unwrap();
    let o = run(&d.0, &["--check"], None);
    assert_eq!(stdout(&o), "A.groovy\n", "each file once, links ignored");
    assert_eq!(o.status.code(), Some(1));
}

#[cfg(unix)]
#[test]
fn hard_links_stay_linked() {
    use std::os::unix::fs::MetadataExt;
    let d = Dir::new("hardlink");
    let h = d.write("H.groovy", UGLY);
    std::fs::hard_link(&h, d.0.join("H2.groovy")).unwrap();
    let ino = std::fs::metadata(&h).unwrap().ino();
    let w = run(&d.0, &["H.groovy"], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(d.read("H.groovy"), PRETTY);
    assert_eq!(d.read("H2.groovy"), PRETTY);
    let m = std::fs::metadata(&h).unwrap();
    assert_eq!((m.ino(), m.nlink()), (ino, 2));
}

#[cfg(unix)]
#[test]
fn symlink_arguments_format_their_target() {
    use std::os::unix::fs::symlink;
    let d = Dir::new("linkarg");
    d.write("real/A.groovy", UGLY);
    d.write("real/B.groovy", UGLY);
    d.write("gen/G.groovy", UGLY);
    d.write(".groovyfmt.toml", "exclude = [\"gen\"]\n");
    symlink("real/A.groovy", d.0.join("link.groovy")).unwrap();
    symlink("link.groovy", d.0.join("chain.groovy")).unwrap();
    symlink("gen/G.groovy", d.0.join("gen.groovy")).unwrap();
    symlink("real/Gone.groovy", d.0.join("dangling.groovy")).unwrap();
    symlink("real", d.0.join("dir")).unwrap();

    let c = run(
        &d.0,
        &["--check", "link.groovy", "chain.groovy", "gen.groovy"],
        None,
    );
    assert_eq!(c.status.code(), Some(1), "{}", stderr(&c));
    assert_eq!(
        stdout(&c),
        "link.groovy\n",
        "named as typed, chain is the same file, excluded target skipped"
    );
    let c = run(&d.0, &["--check", "chain.groovy"], None);
    assert_eq!(stdout(&c), "chain.groovy\n");

    let w = run(&d.0, &["chain.groovy", "gen.groovy"], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(stderr(&w), "reformatted 1 of 1 files\n");
    assert_eq!(d.read("real/A.groovy"), PRETTY);
    assert_eq!(d.read("gen/G.groovy"), UGLY);
    for l in ["link.groovy", "chain.groovy", "gen.groovy"] {
        assert!(
            std::fs::symlink_metadata(d.0.join(l)).unwrap().is_symlink(),
            "{l}"
        );
    }

    let gone = run(&d.0, &["--check", "dangling.groovy"], None);
    assert_eq!(gone.status.code(), Some(2));
    assert_eq!(
        stderr(&gone),
        "groovyfmt: dangling.groovy: no such file or directory\n"
    );

    // a named directory link is walked, as before
    let dir = run(&d.0, &["dir"], None);
    assert_eq!(dir.status.code(), Some(0), "{}", stderr(&dir));
    assert_eq!(d.read("real/B.groovy"), PRETTY);
    assert!(
        std::fs::symlink_metadata(d.0.join("dir"))
            .unwrap()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn symlink_arguments_never_rewrite_outside_the_working_directory() {
    // `groovyfmt *.groovy` in a checkout with a committed link to a file elsewhere
    let d = Dir::new("linkin");
    let out = Dir::new("linkout");
    let target = out.write("Real.groovy", UGLY);
    std::os::unix::fs::symlink(&target, d.0.join("x.groovy")).unwrap();
    d.write("A.groovy", UGLY);
    let w = run(&d.0, &["x.groovy", "A.groovy"], None);
    assert_eq!(w.status.code(), Some(2), "{}", stderr(&w));
    assert!(
        stderr(&w).starts_with(
            "groovyfmt: x.groovy: symlink target is outside the working directory, not formatted\n"
        ),
        "{}",
        stderr(&w)
    );
    assert_eq!(out.read("Real.groovy"), UGLY);
    assert!(
        std::fs::symlink_metadata(d.0.join("x.groovy"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(d.read("A.groovy"), PRETTY, "other files still formatted");
    let c = run(&d.0, &["--check", "x.groovy"], None);
    assert_eq!(c.status.code(), Some(1), "{}", stderr(&c));
    assert_eq!(stdout(&c), "x.groovy\n");
}

#[cfg(unix)]
#[test]
fn symlinked_directory_arguments_never_rewrite_outside_the_working_directory() {
    // `groovyfmt *` in a checkout with a committed `vendor -> /elsewhere`
    let d = Dir::new("dirlinkin");
    let out = Dir::new("dirlinkout");
    out.write("V.groovy", UGLY);
    d.write("real/R.groovy", UGLY);
    std::os::unix::fs::symlink(&out.0, d.0.join("vendor")).unwrap();
    std::os::unix::fs::symlink("real", d.0.join("inside")).unwrap();
    for arg in ["vendor", "vendor/", "vendor/."] {
        let w = run(&d.0, &[arg], None);
        assert_eq!(w.status.code(), Some(2), "{arg}: {}", stderr(&w));
        assert!(
            stderr(&w).starts_with(&format!(
                "groovyfmt: {arg}: symlink target is outside the working directory, not formatted\n"
            )),
            "{}",
            stderr(&w)
        );
        assert_eq!(out.read("V.groovy"), UGLY, "{arg}");
    }
    let c = run(&d.0, &["--check", "vendor"], None);
    assert_eq!(c.status.code(), Some(1), "{}", stderr(&c));
    assert_eq!(stdout(&c), "vendor/V.groovy\n");

    // a link inside the working directory is walked as before
    let w = run(&d.0, &["inside"], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(d.read("real/R.groovy"), PRETTY);
    // an argument that isn't a link is never checked
    let sibling = format!("../{}", out.0.file_name().unwrap().to_str().unwrap());
    let w = run(&d.0, &[&sibling], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(out.read("V.groovy"), PRETTY);
}

#[cfg(unix)]
#[test]
fn write_and_walk_failures_exit_2() {
    use std::os::unix::fs::PermissionsExt;
    let d = Dir::new("perms");
    let ro = d.write("ReadOnly.groovy", UGLY);
    d.write("Ok.groovy", UGLY);
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o444)).unwrap();
    let w = run(&d.0, &[], None);
    assert_eq!(w.status.code(), Some(2), "{}", stderr(&w));
    assert!(stderr(&w).contains("ReadOnly.groovy"), "{}", stderr(&w));
    assert_eq!(d.read("Ok.groovy"), PRETTY, "other files still formatted");

    let locked = d.0.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let c = run(&d.0, &["--check", "Ok.groovy", "locked"], None);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(c.status.code(), Some(2), "{}", stderr(&c));
    assert!(stderr(&c).contains("locked"), "{}", stderr(&c));
}

const TWO: &str = "class A {\n  def f() {\n    println 'x'\n  }\n}\n";

/// The `config:` lines `-v` printed.
fn config_lines(o: &Output) -> Vec<String> {
    stderr(o)
        .lines()
        .filter(|l| l.starts_with("config: "))
        .map(String::from)
        .collect()
}

#[test]
fn config_is_found_from_each_file_not_the_working_directory() {
    let d = Dir::new("cfgfile");
    d.write("proj/.groovyfmt.toml", "indent = 2\n");
    d.write("proj/A.groovy", TWO);
    d.write("elsewhere/keep.txt", "");
    let inside = run(&d.0.join("proj"), &["--check"], None);
    assert_eq!(inside.status.code(), Some(0), "{}", stderr(&inside));
    let elsewhere = d.0.join("elsewhere");
    let c = run(&elsewhere, &["--check", "../proj"], None);
    assert_eq!(c.status.code(), Some(0), "{}", stderr(&c));
    assert_eq!(stdout(&c), "");
    let w = run(&elsewhere, &["../proj", "../proj/A.groovy"], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(stderr(&w), "reformatted 0 of 1 files\n");
    assert_eq!(d.read("proj/A.groovy"), TWO);

    // a file outside the working directory doesn't pick up the working directory's config
    d.write("free/F.groovy", TWO);
    let proj = d.0.join("proj");
    let c = run(&proj, &["--check", "-v", "A.groovy", "../free"], None);
    assert_eq!(c.status.code(), Some(1), "{}", stderr(&c));
    assert_eq!(stdout(&c), "../free/F.groovy\n");
    let cfgs = config_lines(&c);
    assert_eq!(cfgs.len(), 2, "{}", stderr(&c));
    assert!(cfgs[0].ends_with("proj/.groovyfmt.toml"), "{cfgs:?}");
    assert_eq!(cfgs[1], "config: none found, using defaults");
    let w = run(&proj, &["../free/F.groovy"], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(d.read("free/F.groovy"), PRETTY);
}

#[test]
fn nested_config_replaces_its_parent() {
    let d = Dir::new("nested");
    d.write(".groovyfmt.toml", "indent = 4\n");
    d.write("sub/.groovyfmt.toml", "indent = 2\n");
    d.write("A.groovy", UGLY);
    d.write("sub/B.groovy", UGLY);
    d.write("sub/deep/C.groovy", UGLY);
    let w = run(&d.0, &[], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(d.read("A.groovy"), PRETTY);
    assert_eq!(d.read("sub/B.groovy"), TWO);
    assert_eq!(d.read("sub/deep/C.groovy"), TWO);
    let c = run(&d.0, &["--check", "-v"], None);
    assert_eq!(c.status.code(), Some(0), "idempotent: {}", stderr(&c));
    let cfgs = config_lines(&c);
    assert_eq!(cfgs.len(), 2, "{}", stderr(&c));
    assert!(
        cfgs[0].ends_with("/.groovyfmt.toml") && !cfgs[0].contains("sub"),
        "{cfgs:?}"
    );
    assert!(cfgs[1].ends_with("/sub/.groovyfmt.toml"), "{cfgs:?}");

    // --config and --indent override both
    d.write("custom.toml", "indent = 2\n");
    let o = run(&d.0, &["--check", "--config", "custom.toml"], None);
    assert_eq!(stdout(&o), "A.groovy\n", "{}", stderr(&o));
    let o = run(&d.0, &["--check", "--indent", "2"], None);
    assert_eq!(stdout(&o), "A.groovy\n", "{}", stderr(&o));
    let o = run(&d.0, &["--check", "--indent=4"], None);
    assert_eq!(
        stdout(&o),
        "sub/B.groovy\nsub/deep/C.groovy\n",
        "{}",
        stderr(&o)
    );
}

#[test]
fn nested_config_excludes_and_lint_apply_only_inside_it() {
    let d = Dir::new("nestedexcl");
    let import = "import a.b.Unused\n";
    d.write("gen/G.groovy", UGLY);
    d.write("sub/gen/G.groovy", UGLY);
    d.write(
        "sub/.groovyfmt.toml",
        "exclude = [\"gen\"]\n[lint]\nUnusedImport = false\n",
    );
    d.write("I.groovy", import);
    d.write("sub/I.groovy", import);
    let c = run(&d.0, &["--check"], None);
    assert_eq!(stdout(&c), "gen/G.groovy\n", "{}", stderr(&c));
    let named = run(&d.0, &["--check", "sub/gen/G.groovy", "gen/G.groovy"], None);
    assert_eq!(stdout(&named), "gen/G.groovy\n", "{}", stderr(&named));
    let l = run(&d.0, &["--lint"], None);
    assert_eq!(l.status.code(), Some(1), "{}", stderr(&l));
    assert_eq!(
        stdout(&l),
        "I.groovy:1:1: UnusedImport: The [a.b.Unused] import is never referenced (fixable)\n"
    );
}

#[test]
fn excluded_subtree_config_is_never_read() {
    let d = Dir::new("exclsub");
    d.write(".groovyfmt.toml", "exclude = [\"sub/**\", \"vendor\"]\n");
    d.write("sub/.groovyfmt.toml", "indent = \n");
    d.write("sub/S.groovy", UGLY);
    d.write("vendor/x/.groovyfmt.toml", "indent = 2\n");
    d.write("vendor/x/V.groovy", UGLY);
    // nor one that governs no Groovy file
    d.write("docs/.groovyfmt.toml", "indent = \n");
    d.write("docs/notes.txt", "");
    d.write("A.groovy", UGLY);
    let w = run(&d.0, &[], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(stderr(&w), "reformatted 1 of 1 files\n");
    assert_eq!(d.read("A.groovy"), PRETTY);
    assert_eq!(d.read("sub/S.groovy"), UGLY);
    let l = run(&d.0, &["--lint"], None);
    assert_eq!(l.status.code(), Some(0), "{}", stderr(&l));
    // named files (pre-commit) and editor buffers in an excluded subtree with its own config
    let named = ["sub/S.groovy", "vendor/x/V.groovy", "vendor/x", "sub"];
    for arg in named {
        let c = run(&d.0, &["--check", arg], None);
        assert_eq!(c.status.code(), Some(0), "{arg}: {}", stderr(&c));
        assert_eq!(stderr(&c), "would reformat 0 of 0 files\n", "{arg}");
        let o = run(&d.0, &["--stdin", "--stdin-filepath", arg], Some(UGLY));
        assert_eq!(o.status.code(), Some(0), "{arg}: {}", stderr(&o));
        assert_eq!(stdout(&o), UGLY, "{arg}");
    }
    let from_sub = run(&d.0.join("vendor/x"), &["--check", "."], None);
    assert_eq!(stderr(&from_sub), "would reformat 0 of 0 files\n");
    assert_eq!(d.read("vendor/x/V.groovy"), UGLY);
}

#[test]
fn broken_nested_config_governing_a_file_is_fatal_once() {
    let d = Dir::new("brokennested");
    d.write(".groovyfmt.toml", "indent = 4\n");
    d.write("sub/.groovyfmt.toml", "indnet = 2\n");
    d.write("sub/S.groovy", UGLY);
    d.write("sub/T.groovy", UGLY);
    d.write("A.groovy", UGLY);
    let runs: [&[&str]; 3] = [&["--check"], &[], &["--lint"]];
    for args in runs {
        let o = run(&d.0, args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
        let err = stderr(&o);
        assert_eq!(err.matches("unknown key `indnet`").count(), 1, "{err}");
        assert!(err.contains("sub/.groovyfmt.toml: unknown key"), "{err}");
    }
    assert_eq!(d.read("A.groovy"), UGLY, "nothing written");
    // --config replaces discovery, so the broken file isn't read
    let o = run(&d.0, &["--check", "--config", ".groovyfmt.toml"], None);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
}
