//! `groovyfmt --lsp` end to end: scripted sessions against the built binary, each answer
//! compared with what the CLI does for the same text, path and config.

use groovyfmt::json::{self, Value};
use groovyfmt::report::string as js;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// The groovyfmt binary: GROOVYFMT_TEST_BIN when set (test binaries copied to another
/// machine, such as a Windows one, where the build's path doesn't exist), else cargo's.
fn bin() -> String {
    std::env::var("GROOVYFMT_TEST_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_groovyfmt").into())
}

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("groovyfmt-lsp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        // canonical, so the URIs we build match what the server resolves
        Dir(std::fs::canonicalize(&p).unwrap())
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

fn uri(p: &Path) -> String {
    let path = p.to_str().unwrap();
    // Windows: the canonical `\\?\C:\x` is `file:///C:/x`
    #[cfg(windows)]
    let path = &format!("/{}", path.trim_start_matches(r"\\?\").replace('\\', "/"));
    let mut s = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    next_id: u32,
}

impl Server {
    fn start(dir: &Path) -> Server {
        let mut child = Command::new(bin())
            .arg("--lsp")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            loop {
                let mut len = None;
                loop {
                    let mut line = String::new();
                    if out.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    let (k, v) = line.split_once(": ").expect("header");
                    assert_eq!(k, "Content-Length");
                    len = Some(v.parse::<usize>().unwrap());
                }
                let mut body = vec![0; len.expect("Content-Length")];
                out.read_exact(&mut body).unwrap();
                let text = String::from_utf8(body).unwrap();
                let v = json::parse(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
                assert_eq!(v.get("jsonrpc").str(), Some("2.0"), "{text}");
                if tx.send(v).is_err() {
                    return;
                }
            }
        });
        Server {
            stdin: child.stdin.take(),
            child,
            rx,
            next_id: 0,
        }
    }

    fn raw(&mut self, bytes: &[u8]) {
        let _ = self.stdin.as_mut().unwrap().write_all(bytes);
        let _ = self.stdin.as_mut().unwrap().flush();
    }

    fn send(&mut self, body: &str) {
        self.raw(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
    }

    fn notify(&mut self, method: &str, params: &str) {
        self.send(&format!(
            "{{\"jsonrpc\":\"2.0\",\"method\":\"{method}\",\"params\":{params}}}"
        ));
    }

    fn recv(&self) -> Value {
        self.rx
            .recv_timeout(Duration::from_secs(30))
            .expect("no message from the server within 30s")
    }

    /// A request's response; the next message must be it.
    fn request(&mut self, method: &str, params: &str) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}"
        ));
        let v = self.recv();
        assert_eq!(v.get("id").u32(), Some(id), "{v:?}");
        v
    }

    fn result(&mut self, method: &str, params: &str) -> Value {
        let v = self.request(method, params);
        assert!(v.get("error").is_null(), "{method}: {v:?}");
        v.get("result").clone()
    }

    fn initialize(&mut self, encodings: &str) -> Value {
        let r = self.result(
            "initialize",
            &format!(
                "{{\"processId\":null,\"rootUri\":null,\"capabilities\":{{\"general\":{{\"positionEncodings\":{encodings}}}}}}}"
            ),
        );
        self.notify("initialized", "{}");
        r
    }

    fn diagnostics(&self, u: &str) -> Vec<Value> {
        let v = self.recv();
        assert_eq!(
            v.get("method").str(),
            Some("textDocument/publishDiagnostics"),
            "{v:?}"
        );
        assert_eq!(v.get("params").get("uri").str(), Some(u));
        v.get("params").get("diagnostics").arr().to_vec()
    }

    fn open(&mut self, u: &str, text: &str) -> Vec<Value> {
        self.notify(
            "textDocument/didOpen",
            &format!(
                "{{\"textDocument\":{{\"uri\":{},\"languageId\":\"groovy\",\"version\":1,\"text\":{}}}}}",
                js(u),
                js(text)
            ),
        );
        self.diagnostics(u)
    }

    /// Close stdin and wait (bounded) for the exit status.
    fn finish(mut self) -> (Option<i32>, String) {
        drop(self.stdin.take());
        for _ in 0..300 {
            if let Some(st) = self.child.try_wait().unwrap() {
                let mut err = String::new();
                self.child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut err)
                    .unwrap();
                return (st.code(), err);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        panic!("server did not exit within 30s");
    }
}

fn cli(dir: &Path, args: &[&str], stdin: &str) -> (Option<i32>, String) {
    let mut c = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (o.status.code(), String::from_utf8(o.stdout).unwrap())
}

/// Offset of an LSP position in `text`, counting UTF-16 units or bytes.
fn offset(text: &str, p: &Value, utf8: bool) -> usize {
    let (line, ch) = (
        p.get("line").u32().unwrap() as usize,
        p.get("character").u32().unwrap() as usize,
    );
    let mut start = 0;
    for _ in 0..line {
        start += text[start..].find('\n').unwrap() + 1;
    }
    let mut u = 0;
    for (i, c) in text[start..].char_indices() {
        if u >= ch || c == '\n' {
            return start + i;
        }
        u += if utf8 { c.len_utf8() } else { c.len_utf16() };
    }
    text.len()
}

/// `edits` applied to `text`, last first.
fn apply(text: &str, edits: &[Value], utf8: bool) -> String {
    let mut es: Vec<(usize, usize, String)> = edits
        .iter()
        .map(|e| {
            let r = e.get("range");
            (
                offset(text, r.get("start"), utf8),
                offset(text, r.get("end"), utf8),
                e.get("newText").str().unwrap().to_string(),
            )
        })
        .collect();
    es.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut t = text.to_string();
    for (s, e, n) in es {
        t.replace_range(s..e, &n);
    }
    t
}

/// (code, severity, start line, start character, end line, end character)
fn summary(ds: &[Value]) -> Vec<(String, u32, u32, u32, u32, u32)> {
    ds.iter()
        .map(|d| {
            assert_eq!(d.get("source").str(), Some("groovyfmt"));
            assert!(d.get("message").str().is_some_and(|m| !m.is_empty()));
            let r = d.get("range");
            (
                d.get("code").str().unwrap().to_string(),
                d.get("severity").u32().unwrap(),
                r.get("start").get("line").u32().unwrap(),
                r.get("start").get("character").u32().unwrap(),
                r.get("end").get("line").u32().unwrap(),
                r.get("end").get("character").u32().unwrap(),
            )
        })
        .collect()
}

fn s(code: &str, sev: u32, l: u32, c: u32, el: u32, ec: u32) -> (String, u32, u32, u32, u32, u32) {
    (code.to_string(), sev, l, c, el, ec)
}

const MESSY: &str = "import java.util.List\nimport a.Unused\nclass A {\ndef f() {\nprintln '😀😀';   \ntry { g() } catch (Exception e) {}\n\n\n\nreturn 1;\n}\n}\n";

#[test]
fn a_full_session_matches_the_cli() {
    let d = Dir::new("session");
    d.write(
        ".groovyfmt.toml",
        "indent = 2\nexclude = [\"gen/**\"]\n\n[lint]\nEmptyCatchBlock = false\n",
    );
    let path = d.write("src/A.groovy", MESSY);
    let p = path.to_str().unwrap();
    let u = uri(&path);
    let mut srv = Server::start(&d.0);

    let init = srv.initialize("[\"utf-16\"]");
    let caps = init.get("capabilities");
    assert_eq!(caps.get("positionEncoding").str(), Some("utf-16"));
    assert_eq!(caps.get("textDocumentSync").get("change").u32(), Some(1));
    assert_eq!(
        caps.get("textDocumentSync").get("openClose"),
        &Value::Bool(true)
    );
    assert_eq!(caps.get("documentFormattingProvider"), &Value::Bool(true));
    assert_eq!(
        caps.get("documentRangeFormattingProvider"),
        &Value::Bool(true)
    );
    assert_eq!(
        caps.get("codeActionProvider").get("codeActionKinds").arr(),
        [
            Value::Str("quickfix".into()),
            Value::Str("source.fixAll.groovyfmt".into())
        ]
    );
    assert_eq!(init.get("serverInfo").get("name").str(), Some("groovyfmt"));

    // lint violations, as warnings; the `;` after two astral characters is at UTF-16
    // character 14, and the empty catch is off in the config
    let ds = srv.open(&u, MESSY);
    assert_eq!(
        summary(&ds),
        [
            s("UnusedImport", 2, 0, 0, 0, 21),
            s("UnusedImport", 2, 1, 0, 1, 15),
            s("UnnecessarySemicolon", 2, 4, 14, 4, 15),
            s("UnnecessarySemicolon", 2, 9, 8, 9, 9),
        ]
    );
    let (_, lint) = cli(&d.0, &["--lint", "--output-format", "json", p], "");
    assert_eq!(
        lint.matches("\"rule\":").count(),
        ds.len(),
        "the CLI reports the same: {lint}"
    );

    // formatting: one edit that gives byte for byte what --stdin does
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":8,\"insertSpaces\":false}}}}", js(&u)),
    );
    assert_eq!(edits.arr().len(), 1);
    let (code, want) = cli(&d.0, &["--stdin", "--stdin-filepath", p], MESSY);
    assert_eq!(code, Some(0));
    assert_eq!(apply(MESSY, edits.arr(), false), want);
    assert!(
        want.contains("\n  def f() {\n"),
        "indent = 2 from the config"
    );

    // range formatting: as --lines; a selection ending at a line start leaves that line
    let edits = srv.result(
        "textDocument/rangeFormatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"range\":{{\"start\":{{\"line\":3,\"character\":2}},\"end\":{{\"line\":5,\"character\":0}}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&u)),
    );
    let (_, want) = cli(
        &d.0,
        &["--stdin", "--stdin-filepath", p, "--lines", "4-5"],
        MESSY,
    );
    assert_eq!(apply(MESSY, edits.arr(), false), want);
    assert_ne!(want, MESSY);

    // fix all: as --lint --fix on the same text in the same directory
    let copy = d.write("src/B.groovy", MESSY);
    let (code, _) = cli(&d.0, &["--lint", "--fix", copy.to_str().unwrap()], "");
    assert_eq!(code, Some(0), "everything left on is fixable");
    let fixed = std::fs::read_to_string(&copy).unwrap();
    let range0 = "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":0}}";
    let actions = srv.result(
        "textDocument/codeAction",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"range\":{range0},\"context\":{{\"diagnostics\":[],\"only\":[\"source.fixAll\"]}}}}", js(&u)),
    );
    assert_eq!(actions.arr().len(), 1, "{actions:?}");
    let a = &actions.arr()[0];
    assert_eq!(a.get("kind").str(), Some("source.fixAll.groovyfmt"));
    assert_eq!(a.get("title").str(), Some("Fix all auto-fixable problems"));
    let e = a.get("edit").get("changes").get(&u).arr();
    assert_eq!(apply(MESSY, e, false), fixed);
    assert!(!fixed.contains("import") && !fixed.contains(';'), "{fixed}");

    // a quick fix for the diagnostic under the cursor (and no fix-all when only quickfix)
    let actions = srv.result(
        "textDocument/codeAction",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"range\":{{\"start\":{{\"line\":1,\"character\":3}},\"end\":{{\"line\":1,\"character\":3}}}},\"context\":{{\"diagnostics\":[],\"only\":[\"quickfix\"]}}}}", js(&u)),
    );
    assert_eq!(actions.arr().len(), 1, "{actions:?}");
    let a = &actions.arr()[0];
    assert_eq!(a.get("kind").str(), Some("quickfix"));
    assert_eq!(
        summary(a.get("diagnostics").arr()),
        [s("UnusedImport", 2, 1, 0, 1, 15)]
    );
    let e = a.get("edit").get("changes").get(&u).arr();
    assert_eq!(
        apply(MESSY, e, false),
        MESSY.replace("import a.Unused\n", "")
    );
    // no kinds asked for: both kinds, the quick fixes first
    let actions = srv.result(
        "textDocument/codeAction",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"range\":{{\"start\":{{\"line\":9,\"character\":0}},\"end\":{{\"line\":9,\"character\":20}}}},\"context\":{{\"diagnostics\":[]}}}}", js(&u)),
    );
    let kinds: Vec<_> = actions
        .arr()
        .iter()
        .map(|a| a.get("kind").str().unwrap())
        .collect();
    assert_eq!(kinds, ["quickfix", "source.fixAll.groovyfmt"]);

    // the config is read again for each request
    std::fs::write(d.0.join(".groovyfmt.toml"), "indent = 3\n").unwrap();
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&u)),
    );
    assert!(apply(MESSY, edits.arr(), false).contains("\n   def f() {\n"));

    // a change that doesn't parse: errors, and formatting returns nothing
    let broken = "class A {\n  def f() {\n";
    srv.notify(
        "textDocument/didChange",
        &format!("{{\"textDocument\":{{\"uri\":{},\"version\":2}},\"contentChanges\":[{{\"text\":{}}}]}}", js(&u), js(broken)),
    );
    let ds = srv.diagnostics(&u);
    assert_eq!(summary(&ds), [s("ParseError", 1, 1, 10, 1, 11)]);
    let c = d.write("src/C.groovy", broken);
    let (_, syn) = cli(
        &d.0,
        &["--syntax", "--output-format", "json", c.to_str().unwrap()],
        "",
    );
    assert!(
        syn.contains("\"line\":2,\"column\":11,\"end_line\":2,\"end_column\":12"),
        "{syn}"
    );
    assert_eq!(
        ds[0].get("message").str(),
        Some("`{` is never closed at the top level")
    );
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&u)),
    );
    assert_eq!(edits.arr().len(), 0);
    assert_eq!(edits, Value::Arr(vec![]));

    // unknown requests are refused, unknown notifications ignored
    srv.notify("$/cancelRequest", "{\"id\":99}");
    srv.notify("workspace/didChangeConfiguration", "{\"settings\":{}}");
    assert_eq!(summary(&srv.diagnostics(&u)).len(), 1, "republished");
    srv.notify("something/new", "{}");
    let v = srv.request("textDocument/hover", "{}");
    assert_eq!(v.get("error").get("code"), &Value::Num("-32601".into()));

    // close clears the diagnostics
    srv.notify(
        "textDocument/didClose",
        &format!("{{\"textDocument\":{{\"uri\":{}}}}}", js(&u)),
    );
    assert_eq!(srv.diagnostics(&u).len(), 0);
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&u)),
    );
    assert_eq!(edits, Value::Arr(vec![]), "a closed document gets nothing");

    assert_eq!(srv.result("shutdown", "null"), Value::Null);
    let v = srv.request("textDocument/formatting", "{}");
    assert_eq!(v.get("error").get("code"), &Value::Num("-32600".into()));
    srv.notify("exit", "null");
    let (code, err) = srv.finish();
    assert_eq!(code, Some(0), "{err}");
    assert!(!err.contains("panic"), "{err}");
}

#[test]
fn excludes_file_names_and_other_schemes() {
    let d = Dir::new("rules");
    d.write(".groovyfmt.toml", "exclude = [\"gen/**\"]\n");
    let mut srv = Server::start(&d.0);
    srv.initialize("[]");

    // excluded: no diagnostics, no edits
    let gen_path = d.0.join("gen/X.groovy");
    let gen_uri = uri(&gen_path);
    assert_eq!(srv.open(&gen_uri, MESSY).len(), 0);
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&gen_uri)),
    );
    assert_eq!(edits, Value::Arr(vec![]));

    // the Gradle rules follow the file name; percent-encoded names decode
    let script = "repositories {\n    jcenter()\n}\n";
    let gradle = d.0.join("dir with space/build.gradle");
    let codes: Vec<_> = summary(&srv.open(&uri(&gradle), script))
        .into_iter()
        .map(|d| d.0)
        .collect();
    assert_eq!(codes, ["GradleJcenter"]);
    let gradle = d.write("dir with space/build.gradle", script);
    let (_, lint) = cli(
        &d.0,
        &[
            "--lint",
            "--output-format",
            "json",
            gradle.to_str().unwrap(),
        ],
        "",
    );
    assert!(lint.contains("GradleJcenter"), "{lint}");
    let other = d.0.join("dir with space/Other.groovy");
    assert_eq!(srv.open(&uri(&other), script).len(), 0);

    // a broken config: one error at the top, no edits
    d.write("bad/.groovyfmt.toml", "indent = 0\n");
    let bad = uri(&d.0.join("bad/A.groovy"));
    assert_eq!(
        summary(&srv.open(&bad, MESSY)),
        [s("config", 1, 0, 0, 0, 0)]
    );

    // untitled buffers and other schemes are ignored: no diagnostics are published
    srv.notify(
        "textDocument/didOpen",
        &format!("{{\"textDocument\":{{\"uri\":\"untitled:Untitled-1\",\"languageId\":\"groovy\",\"version\":1,\"text\":{}}}}}", js(MESSY)),
    );
    let edits = srv.result(
        "textDocument/formatting",
        "{\"textDocument\":{\"uri\":\"untitled:Untitled-1\"},\"options\":{\"tabSize\":4,\"insertSpaces\":true}}",
    );
    assert_eq!(edits, Value::Arr(vec![]));
    // exit without shutdown
    srv.notify("exit", "null");
    assert_eq!(srv.finish().0, Some(1));
}

#[test]
fn utf8_positions_when_the_client_offers_them() {
    let d = Dir::new("utf8");
    let mut srv = Server::start(&d.0);
    let init = srv.initialize("[\"utf-8\",\"utf-16\"]");
    assert_eq!(
        init.get("capabilities").get("positionEncoding").str(),
        Some("utf-8")
    );
    let path = d.0.join("A.groovy");
    let text = "\tprintln '😀é';\r\n";
    // tab (1) println (7) space (1) ' (1) 😀 (4 bytes) é (2) ' (1): the `;` at byte 17
    assert_eq!(
        summary(&srv.open(&uri(&path), text)),
        [s("UnnecessarySemicolon", 2, 0, 17, 0, 18)]
    );
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&uri(&path))),
    );
    let (_, want) = cli(
        &d.0,
        &["--stdin", "--stdin-filepath", path.to_str().unwrap()],
        text,
    );
    assert_eq!(apply(text, edits.arr(), true), want);
    assert_eq!(srv.result("shutdown", "null"), Value::Null);
    srv.notify("exit", "null");
    assert_eq!(srv.finish().0, Some(0));
}

#[test]
fn normalize_spacing_from_the_config() {
    let d = Dir::new("spacing");
    d.write(".groovyfmt.toml", "normalize_spacing = true\n");
    let text = "def x=1+2\nif(x){\nprintln( x )   \n}\nmap = [a:1,b:2]\n";
    let path = d.0.join("A.groovy");
    let p = path.to_str().unwrap();
    let mut srv = Server::start(&d.0);
    srv.initialize("[]");
    srv.open(&uri(&path), text);
    let opts = "\"options\":{\"tabSize\":4,\"insertSpaces\":true}";
    let edits = srv.result(
        "textDocument/formatting",
        &format!(
            "{{\"textDocument\":{{\"uri\":{}}},{opts}}}",
            js(&uri(&path))
        ),
    );
    let (code, want) = cli(&d.0, &["--stdin", "--stdin-filepath", p], text);
    assert_eq!(code, Some(0));
    assert_eq!(apply(text, edits.arr(), false), want);
    assert!(want.starts_with("def x = 1 + 2\n"), "{want}");
    // range formatting, as --lines
    let edits = srv.result(
        "textDocument/rangeFormatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":3}}}},{opts}}}", js(&uri(&path))),
    );
    let (_, want) = cli(
        &d.0,
        &["--stdin", "--stdin-filepath", p, "--lines", "1-1"],
        text,
    );
    assert_eq!(apply(text, edits.arr(), false), want);
    assert_ne!(want, text);
    assert_eq!(srv.result("shutdown", "null"), Value::Null);
    srv.notify("exit", "null");
    assert_eq!(srv.finish().0, Some(0));
}

#[test]
fn requests_before_initialize_and_twice_initialized() {
    let d = Dir::new("lifecycle");
    let mut srv = Server::start(&d.0);
    let v = srv.request("textDocument/formatting", "{}");
    assert_eq!(v.get("error").get("code"), &Value::Num("-32002".into()));
    srv.initialize("[]");
    let v = srv.request("initialize", "{}");
    assert_eq!(v.get("error").get("code"), &Value::Num("-32600".into()));
    // a string id is echoed as a string
    srv.send("{\"jsonrpc\":\"2.0\",\"id\":\"x\\\"y\",\"method\":\"shutdown\"}");
    let v = srv.recv();
    assert_eq!(v.get("id").str(), Some("x\"y"));
    // end of input after shutdown, without exit
    assert_eq!(srv.finish().0, Some(0));
}

#[test]
fn invalid_json_gets_a_parse_error_and_the_session_goes_on() {
    let d = Dir::new("badjson");
    let mut srv = Server::start(&d.0);
    srv.initialize("[]");
    let path = d.0.join("A.groovy");
    assert_eq!(
        srv.open(&uri(&path), MESSY).len(),
        5,
        "with the empty catch"
    );
    for bad in [
        "{",
        "{\"jsonrpc\":\"2.0\",\"method\":",
        "\u{0}\u{1}",
        "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[",
    ] {
        srv.send(bad);
        // the open document is forgotten (the bad message may have been a change to it)
        let mut got_error = false;
        for _ in 0..2 {
            let v = srv.recv();
            if v.get("method").str() == Some("textDocument/publishDiagnostics") {
                assert_eq!(v.get("params").get("diagnostics").arr().len(), 0);
            } else {
                assert_eq!(
                    v.get("error").get("code"),
                    &Value::Num("-32700".into()),
                    "{v:?}"
                );
                assert!(v.get("id").is_null());
                got_error = true;
                break;
            }
        }
        assert!(got_error);
    }
    // not UTF-8
    srv.raw(b"Content-Length: 2\r\n\r\n\xff\xfe");
    assert_eq!(
        srv.recv().get("error").get("code"),
        &Value::Num("-32700".into())
    );
    // valid JSON that isn't a request
    srv.send("[1,2]");
    assert_eq!(
        srv.recv().get("error").get("code"),
        &Value::Num("-32600".into())
    );
    srv.send("{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"shutdown\"}");
    assert_eq!(
        srv.recv().get("error").get("code"),
        &Value::Num("-32600".into())
    );
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&uri(&path))),
    );
    assert_eq!(edits, Value::Arr(vec![]), "forgotten until sent again");
    // the next full change brings it back
    srv.notify(
        "textDocument/didChange",
        &format!(
            "{{\"textDocument\":{{\"uri\":{},\"version\":3}},\"contentChanges\":[{{\"text\":{}}}]}}",
            js(&uri(&path)),
            js(MESSY)
        ),
    );
    assert_eq!(
        srv.diagnostics(&uri(&path)).len(),
        5,
        "with the empty catch"
    );
    let edits = srv.result(
        "textDocument/formatting",
        &format!("{{\"textDocument\":{{\"uri\":{}}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}", js(&uri(&path))),
    );
    assert_eq!(edits.arr().len(), 1);
    assert_eq!(srv.result("shutdown", "null"), Value::Null);
    srv.notify("exit", "null");
    let (code, err) = srv.finish();
    assert_eq!(code, Some(0));
    assert!(!err.contains("panic"), "{err}");
}

#[test]
fn garbage_bytes_end_the_session_cleanly() {
    let d = Dir::new("garbage");
    for junk in [
        &b"\x00\xff\xfe garbage \x1b[2J\r\n\r\n"[..],
        b"Content-Length: banana\r\n\r\n{}",
        b"GET / HTTP/1.1\r\nHost: x\r\n\r\n",
        &[b'A'; 100_000][..],
    ] {
        let mut srv = Server::start(&d.0);
        srv.raw(junk);
        let (code, err) = srv.finish();
        assert_eq!(code, Some(1), "{err}");
        assert!(!err.contains("panic"), "{err}");
    }
}
