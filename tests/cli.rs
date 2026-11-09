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

    let w = run(&d.0, &[], None);
    assert_eq!(w.status.code(), Some(0), "{}", stderr(&w));
    assert_eq!(d.read("src/A.groovy"), PRETTY);
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
