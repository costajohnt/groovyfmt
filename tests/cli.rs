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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.unwrap_or("").as_bytes())
        .unwrap();
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
        "I.groovy:1:1: UnusedImport: The [a.b.Unused] import is never referenced\n"
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
