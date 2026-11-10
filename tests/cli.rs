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
