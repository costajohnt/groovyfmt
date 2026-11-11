//! `--lsp`: a Language Server Protocol server on stdin/stdout, for editors and coding agents.
//! It formats, range-formats, publishes parse errors and lint violations, and offers the
//! `--lint --fix` fixes as code actions, each exactly as the CLI would for the same text and
//! file path (`--stdin --stdin-filepath`, `--lines`, `--lint`, `--syntax`, `--lint --fix`).
//!
//! JSON-RPC 2.0 with `Content-Length` framing, one message at a time; logs go to stderr. The
//! document text comes only from the client; the file path in its `file:` URI is used only to
//! find the config and to pick the Jenkins, Gradle and Spock rules. Nothing is written to
//! disk and nothing is run. The config is looked up afresh for every request and every
//! diagnostics run, so an edited `.groovyfmt.toml` applies from the next one.

use crate::config::{Config, Resolver};
use crate::fix::{self, Fix};
use crate::format::{Outcome, format};
use crate::json::{self, Value};
use crate::report::string as js;
use crate::{MAX_FILE_BYTES, diff, lint, syntax};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// A message body larger than this is skipped unread. A 10 MB document escaped as JSON fits.
const MAX_MESSAGE: usize = 64 << 20;
/// Header lines are short; this bounds the memory a stream without line breaks can take.
const MAX_HEADER_LINE: u64 = 4096;
const MAX_HEADERS: usize = 32;
/// Quick fixes offered per code action request; each one re-lints the file.
const MAX_QUICK_FIXES: usize = 10;

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;
const SERVER_NOT_INITIALIZED: i32 = -32002;

const FIX_ALL: &str = "source.fixAll.groovyfmt";

#[derive(Debug, PartialEq)]
enum Frame {
    Body(Vec<u8>),
    /// Over `MAX_MESSAGE`; its bytes were read and dropped.
    TooLarge,
}

/// The next message: `Ok(None)` at a clean end of input, `Err` when the framing is broken,
/// after which the stream can't be trusted to line up again.
fn read_frame(r: &mut impl BufRead) -> Result<Option<Frame>, String> {
    let mut len = None;
    let mut line = Vec::new();
    for n in 0.. {
        if n > MAX_HEADERS {
            return Err("too many header lines".into());
        }
        line.clear();
        r.by_ref()
            .take(MAX_HEADER_LINE)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if line.is_empty() && n == 0 {
            return Ok(None);
        }
        if line.last() != Some(&b'\n') {
            return Err("header line too long, or input ended inside the headers".into());
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        let Some((name, value)) = text.split_once(':') else {
            return Err(format!("bad header line `{}`", text.escape_debug()));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            len = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| format!("bad Content-Length `{}`", value.escape_debug()))?,
            );
        }
    }
    let len = len.ok_or("missing Content-Length")?;
    if len > MAX_MESSAGE {
        let read = std::io::copy(&mut r.by_ref().take(len as u64), &mut std::io::sink())
            .map_err(|e| e.to_string())?;
        return if read == len as u64 {
            Ok(Some(Frame::TooLarge))
        } else {
            Err("input ended inside a message".into())
        };
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)
        .map_err(|_| "input ended inside a message".to_string())?;
    Ok(Some(Frame::Body(body)))
}

fn log(msg: &str) {
    let msg: String = msg
        .chars()
        .map(|c| {
            if c.is_control() {
                c.escape_debug().to_string()
            } else {
                c.to_string()
            }
        })
        .collect();
    eprintln!("groovyfmt --lsp: {msg}");
}

fn send(body: &str) {
    let mut out = std::io::stdout().lock();
    let sent =
        write!(out, "Content-Length: {}\r\n\r\n{body}", body.len()).and_then(|_| out.flush());
    if let Err(e) = sent {
        log(&format!("writing to the client: {e}"));
        std::process::exit(1);
    }
}

/// The path of a `file:` URI: `file:///p` or `file://localhost/p`, percent-decoded. `None`
/// for any other scheme, a remote host, or a malformed escape.
fn uri_path(uri: &str) -> Option<PathBuf> {
    let scheme = uri.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = uri[7..].split(['?', '#']).next()?;
    let (host, path) = rest.split_at(rest.find('/')?);
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut it = path.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = [it.next()?, it.next()?];
            let hex = std::str::from_utf8(&hex).ok()?;
            bytes.push(
                u8::from_str_radix(hex, 16)
                    .ok()
                    .filter(|_| hex.bytes().all(|c| c.is_ascii_hexdigit()))?,
            );
        } else {
            bytes.push(b);
        }
    }
    if bytes.contains(&0) {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        let s = String::from_utf8(bytes).ok()?;
        // `/C:/x` names `C:/x`
        let drive =
            s.len() >= 3 && s.as_bytes()[2] == b':' && s.as_bytes()[1].is_ascii_alphabetic();
        Some(PathBuf::from(if drive { &s[1..] } else { &s[..] }))
    }
}

/// Positions in one text, between byte offsets, groovyfmt's (line, column) (1-based, lines
/// end at `\n`, columns count characters) and LSP's (0-based, lines end at `\n`, `\r\n` or
/// `\r`, characters counted in UTF-16 or UTF-8 code units as negotiated).
struct Lines<'a> {
    text: &'a str,
    /// Where each `\n`-terminated line starts.
    nl: Vec<usize>,
    /// Where each LSP line starts.
    lsp: Vec<usize>,
    utf8: bool,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str, utf8: bool) -> Lines<'a> {
        let b = text.as_bytes();
        let nl = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let lsp = std::iter::once(0)
            .chain(b.iter().enumerate().filter_map(|(i, &c)| {
                let ends = c == b'\n' || (c == b'\r' && b.get(i + 1) != Some(&b'\n'));
                ends.then_some(i + 1)
            }))
            .collect();
        Lines {
            text,
            nl,
            lsp,
            utf8,
        }
    }

    fn units(&self, s: &str) -> usize {
        if self.utf8 {
            s.len()
        } else {
            s.encode_utf16().count()
        }
    }

    /// The offset of a groovyfmt position, clamped to its line.
    fn offset(&self, line: usize, col: usize) -> usize {
        let Some(&start) = self.nl.get(line.saturating_sub(1)) else {
            return self.text.len();
        };
        let end = self.nl.get(line).map_or(self.text.len(), |n| n - 1);
        self.text[start..end]
            .char_indices()
            .nth(col.saturating_sub(1))
            .map_or(end, |(i, _)| start + i)
    }

    /// The 1-based groovyfmt line holding `offset`.
    fn line_of(&self, offset: usize) -> usize {
        self.nl.partition_point(|&s| s <= offset)
    }

    /// The LSP position of `offset`, a char boundary, as JSON.
    fn pos(&self, offset: usize) -> String {
        let line = self.lsp.partition_point(|&s| s <= offset) - 1;
        let mut seg = &self.text[self.lsp[line]..offset];
        // inside a `\r\n`: the end of the line
        if self.text[offset..].starts_with('\n') {
            seg = seg.strip_suffix('\r').unwrap_or(seg);
        }
        format!("{{\"line\":{line},\"character\":{}}}", self.units(seg))
    }

    fn range(&self, start: usize, end: usize) -> String {
        format!(
            "{{\"start\":{},\"end\":{}}}",
            self.pos(start),
            self.pos(end)
        )
    }

    /// The offset of an LSP position; past the end of a line is its end, past the last line
    /// the end of the text.
    fn offset_of(&self, p: &Value) -> Option<usize> {
        let (line, ch) = (
            p.get("line").u32()? as usize,
            p.get("character").u32()? as usize,
        );
        let Some(&start) = self.lsp.get(line) else {
            return Some(self.text.len());
        };
        let end = self.lsp.get(line + 1).map_or(self.text.len(), |&n| {
            n - if self.text[..n].ends_with("\r\n") {
                2
            } else {
                1
            }
        });
        let mut u = 0;
        for (i, c) in self.text[start..end].char_indices() {
            if u >= ch {
                return Some(start + i);
            }
            u += if self.utf8 {
                c.len_utf8()
            } else {
                c.len_utf16()
            };
        }
        Some(end)
    }
}

/// One edit turning `old` into `new`, as a JSON array of TextEdits: the lines from the first
/// that differs to the last that differs are replaced, so the cursor and folds elsewhere stay.
fn edits(lines: &Lines, new: &str) -> String {
    let old = lines.text;
    let (a, b): (Vec<&str>, Vec<&str>) = (
        old.split_inclusive('\n').collect(),
        new.split_inclusive('\n').collect(),
    );
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..]
        .iter()
        .rev()
        .zip(b[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let start: usize = a[..pre].iter().map(|l| l.len()).sum();
    let end = old.len() - a[a.len() - suf..].iter().map(|l| l.len()).sum::<usize>();
    let text = b[pre..b.len() - suf].concat();
    format!(
        "[{{\"range\":{},\"newText\":{}}}]",
        lines.range(start, end),
        js(&text)
    )
}

fn non_ws(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|c| !c.is_whitespace())
}

struct Doc {
    path: PathBuf,
    text: String,
    /// As the client sent it, a JSON integer.
    version: Option<String>,
}

/// What applies to a document.
enum Setup {
    /// Excluded by the config, or over the size cap: no diagnostics, no edits.
    Off,
    Cfg(Arc<Config>),
    /// A config file that applies could not be read.
    Broken(String),
}

struct Server {
    fixed: Option<Config>,
    indent: Option<usize>,
    initialized: bool,
    shutdown: bool,
    utf8: bool,
    /// Open `file:` documents by URI.
    docs: HashMap<String, Doc>,
}

type Reply = Result<String, (i32, String)>;

impl Server {
    fn setup(&self, doc: &Doc) -> Setup {
        if doc.text.len() as u64 > MAX_FILE_BYTES {
            return Setup::Off;
        }
        // ponytail: a fresh resolver per call re-reads the configs above the file each time
        // (a few stats and small reads), so an edited config applies without file watching
        let mut res = Resolver::new(self.fixed.clone(), self.indent);
        match res.excluded(&doc.path).and_then(|ex| {
            if ex {
                Ok(None)
            } else {
                res.of(&doc.path).map(Some)
            }
        }) {
            Ok(None) => Setup::Off,
            Ok(Some(c)) => Setup::Cfg(c),
            Err(e) => Setup::Broken(e),
        }
    }

    fn publish(&self, uri: &str) {
        let Some(doc) = self.docs.get(uri) else {
            return;
        };
        let lines = Lines::new(&doc.text, self.utf8);
        let diag = |start, end, severity, code: &str, message: &str| {
            format!(
                "{{\"range\":{},\"severity\":{severity},\"code\":{},\"source\":\"groovyfmt\",\"message\":{}}}",
                lines.range(start, end),
                js(code),
                js(message)
            )
        };
        let ds: Vec<String> = match self.setup(doc) {
            Setup::Off => Vec::new(),
            Setup::Broken(e) => vec![diag(0, 0, 1, "config", &e)],
            Setup::Cfg(cfg) => match lint::lint_file(&doc.path, &doc.text) {
                Ok(vs) => vs
                    .iter()
                    .filter(|v| cfg.rule_enabled(v.rule))
                    .map(|v| {
                        let (s, e) = (
                            lines.offset(v.line, v.col),
                            lines.offset(v.end_line, v.end_col),
                        );
                        diag(s, e, 2, v.rule, &v.message)
                    })
                    .collect(),
                Err(_) => syntax::check(&doc.text)
                    .0
                    .iter()
                    .map(|e| {
                        let (s, end) = (
                            lines.offset(e.line, e.col),
                            lines.offset(e.end_line, e.end_col),
                        );
                        diag(s, end, 1, crate::report::PARSE_RULE, &e.message)
                    })
                    .collect(),
            },
        };
        self.notify_diagnostics(uri, doc.version.as_deref(), &ds.join(","));
    }

    fn notify_diagnostics(&self, uri: &str, version: Option<&str>, diagnostics: &str) {
        let version = version.map_or(String::new(), |v| format!(",\"version\":{v}"));
        send(&format!(
            "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{{\"uri\":{}{version},\"diagnostics\":[{diagnostics}]}}}}",
            js(uri)
        ));
    }

    /// The open document a request names, with its config; `None` if there is nothing to do.
    fn doc(&self, params: &Value) -> Option<(&Doc, Arc<Config>)> {
        let doc = self
            .docs
            .get(params.get("textDocument").get("uri").str()?)?;
        match self.setup(doc) {
            Setup::Cfg(c) => Some((doc, c)),
            _ => None,
        }
    }

    fn formatting(&self, params: &Value, range: bool) -> Reply {
        let Some((doc, cfg)) = self.doc(params) else {
            return Ok("[]".into());
        };
        let lines = Lines::new(&doc.text, self.utf8);
        let out = if range {
            let r = params.get("range");
            let (Some(s), Some(e)) = (
                lines.offset_of(r.get("start")),
                lines.offset_of(r.get("end")),
            ) else {
                return Err((INVALID_PARAMS, "bad range".into()));
            };
            let first = lines.line_of(s);
            // a selection ending at the start of a line doesn't take that line
            let at_line_start = e > s && lines.nl.binary_search(&e).is_ok();
            let last = (lines.line_of(e) - usize::from(at_line_start)).max(first);
            diff::format_lines(&doc.text, &cfg.style, &[(first, last)])
        } else {
            format(&doc.text, &cfg.style)
        };
        Ok(match out {
            // format checked the tokens; this re-checks the one promise an edit must keep:
            // only whitespace changes (normalize_spacing may add some between tokens)
            Outcome::Formatted(new) if non_ws(&doc.text).eq(non_ws(&new)) => edits(&lines, &new),
            _ => "[]".into(),
        })
    }

    fn code_actions(&self, params: &Value) -> Reply {
        let Some((doc, cfg)) = self.doc(params) else {
            return Ok("[]".into());
        };
        let only: Vec<&str> = params
            .get("context")
            .get("only")
            .arr()
            .iter()
            .filter_map(Value::str)
            .collect();
        let want = |kind: &str| {
            only.is_empty()
                || only
                    .iter()
                    .any(|o| kind == *o || kind.strip_prefix(o).is_some_and(|r| r.starts_with('.')))
        };
        let Ok(vs) = lint::lint_file(&doc.path, &doc.text) else {
            return Ok("[]".into());
        };
        let lines = Lines::new(&doc.text, self.utf8);
        let uri = js(params.get("textDocument").get("uri").str().unwrap_or(""));
        let action = |title: &str, kind: &str, new: &str, extra: &str| {
            format!(
                "{{\"title\":{},\"kind\":{}{extra},\"edit\":{{\"changes\":{{{uri}:{}}}}}}}",
                js(title),
                js(kind),
                edits(&lines, new)
            )
        };
        let mut out = Vec::new();
        if want("quickfix") {
            let r = params.get("range");
            let (Some(rs), Some(re)) = (
                lines.offset_of(r.get("start")),
                lines.offset_of(r.get("end")),
            ) else {
                return Err((INVALID_PARAMS, "bad range".into()));
            };
            for v in vs
                .iter()
                .filter(|v| v.fix.is_some() && cfg.rule_enabled(v.rule))
                .filter(|v| {
                    let (s, e) = (
                        lines.offset(v.line, v.col),
                        lines.offset(v.end_line, v.end_col),
                    );
                    s <= re && rs <= e
                })
                .take(MAX_QUICK_FIXES)
            {
                // ponytail: "this violation" is matched by rule, place and message, so if
                // fixing it exposes an identical one where it was (a third copy of an
                // import), that one goes too; fix's checks still apply to the result
                let this = |w: &lint::Violation| {
                    (w.rule, w.line, w.col, &w.message) == (v.rule, v.line, v.col, &v.message)
                };
                if let Fix::Fixed(new, _) = fix::fix(&doc.path, &doc.text, this) {
                    let (s, e) = (
                        lines.offset(v.line, v.col),
                        lines.offset(v.end_line, v.end_col),
                    );
                    let diag = format!(
                        ",\"diagnostics\":[{{\"range\":{},\"severity\":2,\"code\":{},\"source\":\"groovyfmt\",\"message\":{}}}],\"isPreferred\":true",
                        lines.range(s, e),
                        js(v.rule),
                        js(&v.message)
                    );
                    out.push(action(
                        &format!("Fix {}: {}", v.rule, v.message),
                        "quickfix",
                        &new,
                        &diag,
                    ));
                }
            }
        }
        if want(FIX_ALL)
            && let Fix::Fixed(new, _) = fix::fix(&doc.path, &doc.text, |v| cfg.rule_enabled(v.rule))
        {
            out.push(action("Fix all auto-fixable problems", FIX_ALL, &new, ""));
        }
        Ok(format!("[{}]", out.join(",")))
    }

    fn open(&mut self, uri: &str, text: String, version: &Value) {
        let Some(path) = uri_path(uri) else {
            return;
        };
        let version = match version {
            Value::Num(n) => Some(n.clone()),
            _ => None,
        };
        self.docs.insert(
            uri.to_string(),
            Doc {
                path,
                text,
                version,
            },
        );
        self.publish(uri);
    }

    fn initialize(&mut self, params: &Value) -> Reply {
        self.utf8 = params
            .get("capabilities")
            .get("general")
            .get("positionEncodings")
            .arr()
            .iter()
            .any(|e| e.str() == Some("utf-8"));
        self.initialized = true;
        Ok(format!(
            "{{\"capabilities\":{{\"positionEncoding\":\"{}\",\"textDocumentSync\":{{\"openClose\":true,\"change\":1,\"save\":true}},\"documentFormattingProvider\":true,\"documentRangeFormattingProvider\":true,\"codeActionProvider\":{{\"codeActionKinds\":[\"quickfix\",\"{FIX_ALL}\"]}}}},\"serverInfo\":{{\"name\":\"groovyfmt\",\"version\":\"{}\"}}}}",
            if self.utf8 { "utf-8" } else { "utf-16" },
            env!("CARGO_PKG_VERSION")
        ))
    }

    fn request(&mut self, method: &str, params: &Value) -> Reply {
        if self.shutdown {
            return Err((INVALID_REQUEST, "the server is shut down".into()));
        }
        if method == "initialize" {
            return if self.initialized {
                Err((INVALID_REQUEST, "already initialized".into()))
            } else {
                self.initialize(params)
            };
        }
        if !self.initialized {
            return Err((SERVER_NOT_INITIALIZED, "not initialized".into()));
        }
        match method {
            "shutdown" => {
                self.shutdown = true;
                Ok("null".into())
            }
            "textDocument/formatting" => self.formatting(params, false),
            "textDocument/rangeFormatting" => self.formatting(params, true),
            "textDocument/codeAction" => self.code_actions(params),
            _ => Err((METHOD_NOT_FOUND, format!("unknown method {method}"))),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) {
        if !self.initialized || self.shutdown {
            return;
        }
        let uri = params
            .get("textDocument")
            .get("uri")
            .str()
            .unwrap_or("")
            .to_string();
        match method {
            "textDocument/didOpen" => {
                let td = params.get("textDocument");
                if let Some(text) = td.get("text").str() {
                    self.open(&uri, text.to_string(), td.get("version"));
                }
            }
            "textDocument/didChange" => {
                // full sync: the last change is the whole text, which also brings back a
                // document `forget_all` dropped; a ranged change (which this server never
                // asked for) can't be applied, so the document is dropped
                match params.get("contentChanges").arr().last() {
                    Some(c) if c.get("range").is_null() && c.get("text").str().is_some() => {
                        let text = c.get("text").str().unwrap_or("").to_string();
                        self.open(&uri, text, params.get("textDocument").get("version"));
                    }
                    _ => {
                        self.docs.remove(&uri);
                        self.notify_diagnostics(&uri, None, "");
                    }
                }
            }
            "textDocument/didClose" => {
                if self.docs.remove(&uri).is_some() {
                    self.notify_diagnostics(&uri, None, "");
                }
            }
            "textDocument/didSave" => self.publish(&uri),
            // a config may have changed: diagnostics again for every open document
            "workspace/didChangeConfiguration" | "workspace/didChangeWatchedFiles" => {
                let uris: Vec<String> = self.docs.keys().cloned().collect();
                for u in uris {
                    self.publish(&u);
                }
            }
            _ => {}
        }
    }

    /// A message we could not read may have been a change to a document, so every document
    /// is forgotten (and its diagnostics cleared) until the client sends it again; no edit is
    /// ever computed against text the client no longer has.
    fn forget_all(&mut self) {
        for (uri, _) in std::mem::take(&mut self.docs) {
            self.notify_diagnostics(&uri, None, "");
        }
    }
}

fn respond(id: &str, reply: Reply) {
    send(&match reply {
        Ok(result) => format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{result}}}"),
        Err((code, message)) => format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{{\"code\":{code},\"message\":{}}}}}",
            js(&message)
        ),
    });
}

/// Serve until `exit` or the end of input: exit status 0 after `shutdown`, else 1.
pub fn run(fixed: Option<Config>, indent: Option<usize>) -> i32 {
    let mut s = Server {
        fixed,
        indent,
        initialized: false,
        shutdown: false,
        utf8: false,
        docs: HashMap::new(),
    };
    let mut input = std::io::stdin().lock();
    loop {
        let body = match read_frame(&mut input) {
            Ok(Some(Frame::Body(b))) => b,
            Ok(Some(Frame::TooLarge)) => {
                log(&format!("skipped a message over {MAX_MESSAGE} bytes"));
                s.forget_all();
                continue;
            }
            Ok(None) => return i32::from(!s.shutdown),
            Err(e) => {
                log(&e);
                return 1;
            }
        };
        let msg = match std::str::from_utf8(&body)
            .map_err(|e| e.to_string())
            .and_then(json::parse)
        {
            Ok(m) => m,
            Err(e) => {
                log(&format!("unreadable message: {e}"));
                s.forget_all();
                respond("null", Err((PARSE_ERROR, e)));
                continue;
            }
        };
        let id = match msg.get("id") {
            Value::Num(n) => Some(n.clone()),
            Value::Str(t) => Some(js(t)),
            Value::Null => None,
            _ => {
                respond("null", Err((INVALID_REQUEST, "bad id".into())));
                continue;
            }
        };
        let Some(method) = msg.get("method").str() else {
            // a response to a request we never send, or not a message at all
            if msg.get("result").is_null() && msg.get("error").is_null() {
                respond(
                    id.as_deref().unwrap_or("null"),
                    Err((INVALID_REQUEST, "no method".into())),
                );
            }
            continue;
        };
        if method == "exit" {
            return i32::from(!s.shutdown);
        }
        let params = msg.get("params");
        // a bug in formatting or linting one document fails that request, not the server
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &id {
            Some(_) => Some(s.request(method, params)),
            None => {
                s.notification(method, params);
                None
            }
        }));
        match (id, result) {
            (Some(id), Ok(Some(reply))) => respond(&id, reply),
            (Some(id), Err(_)) => respond(
                &id,
                Err((INTERNAL_ERROR, "internal error, please report".into())),
            ),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(input: &[u8]) -> Vec<Result<Option<Frame>, String>> {
        // one byte per read, as a slow pipe delivers it
        struct Slow<'a>(&'a [u8]);
        impl Read for Slow<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.0.len().min(buf.len()).min(1);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let mut r = std::io::BufReader::with_capacity(3, Slow(input));
        let mut out = Vec::new();
        loop {
            let f = read_frame(&mut r);
            let stop = !matches!(f, Ok(Some(_)));
            out.push(f);
            if stop {
                return out;
            }
        }
    }

    fn body(s: &str) -> Result<Option<Frame>, String> {
        Ok(Some(Frame::Body(s.as_bytes().to_vec())))
    }

    #[test]
    fn framing() {
        // two messages in one read, split across reads, other headers, any header case
        let input = b"Content-Length: 2\r\n\r\n{}content-length:3\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n[1]";
        assert_eq!(frames(input), [body("{}"), body("[1]"), Ok(None)]);
        // the length counts bytes, not characters
        let input = "Content-Length: 4\r\n\r\n\"é\"".as_bytes();
        assert_eq!(frames(input), [body("\"é\""), Ok(None)]);
        assert_eq!(frames(b""), [Ok(None)]);
    }

    #[test]
    fn broken_framing_is_an_error() {
        for input in [
            &b"Content-Type: x\r\n\r\n{}"[..],
            b"Content-Length: x\r\n\r\n{}",
            b"Content-Length: -1\r\n\r\n{}",
            b"Content-Length: 99999999999999999999999\r\n\r\n",
            b"garbage\r\n\r\n",
            b"\r\n",
            b"Content-Length: 5\r\n\r\n{}",
            b"Content-Length: 5\r\n",
        ] {
            let got = frames(input);
            assert!(
                got.last().unwrap().is_err(),
                "{:?}: {got:?}",
                String::from_utf8_lossy(input)
            );
        }
        // no line break for megabytes: refused after one header line's worth
        assert!(frames(&vec![b'a'; 1 << 20])[0].is_err());
        let many: Vec<u8> = b"X: y\r\n".repeat(100);
        assert!(frames(&many)[0].is_err());
    }

    #[test]
    fn huge_messages_are_skipped_not_buffered() {
        let mut input = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE + 1).into_bytes();
        input.resize(input.len() + MAX_MESSAGE + 1, b'x');
        input.extend(b"Content-Length: 2\r\n\r\n{}");
        let mut r = std::io::BufReader::new(&input[..]);
        assert_eq!(read_frame(&mut r), Ok(Some(Frame::TooLarge)));
        assert_eq!(read_frame(&mut r), body("{}"));
        // a huge length with the input ending early
        let mut r = std::io::BufReader::new(&b"Content-Length: 999999999999\r\n\r\nabc"[..]);
        assert!(read_frame(&mut r).is_err());
    }

    #[test]
    fn file_uris() {
        let p = |u: &str| uri_path(u).map(|p| p.to_string_lossy().into_owned());
        #[cfg(unix)]
        {
            assert_eq!(
                p("file:///home/a/B.groovy").as_deref(),
                Some("/home/a/B.groovy")
            );
            assert_eq!(
                p("file:///a%20b/%C3%A9%23x.gradle").as_deref(),
                Some("/a b/é#x.gradle")
            );
            assert_eq!(
                p("FILE://localhost/x/Jenkinsfile").as_deref(),
                Some("/x/Jenkinsfile")
            );
            assert_eq!(p("file:///x/A.groovy#frag").as_deref(), Some("/x/A.groovy"));
            assert_eq!(p("file:///c%3A/x").as_deref(), Some("/c:/x"));
            // bytes that aren't UTF-8 are a valid file name on unix
            assert_eq!(uri_path("file:///x%FF").unwrap().as_os_str().len(), 3);
        }
        for bad in [
            "untitled:Untitled-1",
            "https://example.com/A.groovy",
            "file://server/share/A.groovy",
            "file:A.groovy",
            "file://",
            "file:///a%2",
            "file:///a%zz",
            "file:///a%+1",
            "file:///a%00b",
            "file",
            "",
        ] {
            assert_eq!(uri_path(bad), None, "{bad}");
        }
    }

    fn lsp_pos(l: &Lines, off: usize) -> (u32, u32) {
        let v = json::parse(&l.pos(off)).unwrap();
        (
            v.get("line").u32().unwrap(),
            v.get("character").u32().unwrap(),
        )
    }

    fn at(line: u32, ch: u32) -> Value {
        json::parse(&format!("{{\"line\":{line},\"character\":{ch}}}")).unwrap()
    }

    #[test]
    fn positions_in_utf16_and_utf8() {
        // tab, BMP and astral characters; a CRLF line and a lone CR line
        let text = "\tx = '😀é'\r\ny\rz😀\n";
        let (u16, u8) = (Lines::new(text, false), Lines::new(text, true));
        // groovyfmt's line 1 column 8 is the `é` (characters: tab x space = space ' 😀 é)
        let e = u16.offset(1, 8);
        assert_eq!(&text[e..e + 2], "é");
        assert_eq!(lsp_pos(&u16, e), (0, 8), "😀 is two UTF-16 units");
        assert_eq!(lsp_pos(&u8, e), (0, 10), "and four UTF-8 bytes");
        assert_eq!(u16.offset_of(&at(0, 8)), Some(e));
        assert_eq!(u8.offset_of(&at(0, 10)), Some(e));
        // the end of line 1 is before the `\r`, and past it is clamped there
        let eol = text.find('\r').unwrap();
        assert_eq!(
            u16.offset(1, 99),
            eol + 1,
            "groovyfmt lines end at \\n, the \\r is in it"
        );
        assert_eq!(
            lsp_pos(&u16, eol + 1),
            (0, 10),
            "inside \\r\\n is the end of the line"
        );
        assert_eq!(u16.offset_of(&at(0, 99)), Some(eol));
        // the lone `\r` starts a new LSP line, not a groovyfmt one
        let z = text.find('z').unwrap();
        assert_eq!(lsp_pos(&u16, z), (2, 0));
        assert_eq!(u16.line_of(z), 2);
        assert_eq!(u16.offset_of(&at(2, 3)), Some(text.len() - 1));
        assert_eq!(lsp_pos(&u16, text.len()), (3, 0));
        assert_eq!(u16.offset_of(&at(9, 0)), Some(text.len()));
        // a position inside a surrogate pair lands on a char boundary
        let off = u16.offset_of(&at(2, 2)).unwrap();
        assert!(text.is_char_boundary(off));
        assert_eq!(
            u16.offset_of(&json::parse("{\"line\":-1,\"character\":0}").unwrap()),
            None
        );
    }

    #[test]
    fn edits_replace_only_the_changed_lines() {
        let old = "a\r\n  b\nc\n😀d  \ne\n";
        let new = "a\r\nb\nc\n😀d\ne\n";
        let l = Lines::new(old, false);
        let v = json::parse(&edits(&l, new)).unwrap();
        let e = &v.arr()[0];
        let r = e.get("range");
        assert_eq!(
            (
                r.get("start").get("line").u32(),
                r.get("start").get("character").u32()
            ),
            (Some(1), Some(0))
        );
        assert_eq!(
            (
                r.get("end").get("line").u32(),
                r.get("end").get("character").u32()
            ),
            (Some(4), Some(0))
        );
        assert_eq!(e.get("newText").str(), Some("b\nc\n😀d\n"));
        // to and from empty, and a missing final newline
        for (old, new) in [("", "x\n"), ("x\n", ""), ("x", "x\n"), ("a\nb", "a\nb\n")] {
            let l = Lines::new(old, false);
            let v = json::parse(&edits(&l, new)).unwrap();
            let e = &v.arr()[0];
            let s = l.offset_of(e.get("range").get("start")).unwrap();
            let t = l.offset_of(e.get("range").get("end")).unwrap();
            let applied = format!(
                "{}{}{}",
                &old[..s],
                e.get("newText").str().unwrap(),
                &old[t..]
            );
            assert_eq!(applied, new, "{old:?} -> {new:?}");
        }
    }
}
