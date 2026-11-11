pub mod baseline;
pub mod config;
pub mod diff;
pub mod fix;
pub mod format;
pub mod json;
pub mod lint;
pub mod lsp;
pub mod report;
mod spacing;
pub mod syntax;
pub mod wrap;

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};
use tree_sitter_language::LanguageFn;

/// Files larger than this are skipped: generated or vendored code, and a guard against
/// a crafted input exhausting memory. `--lsp` leaves documents this large alone too.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

unsafe extern "C" {
    fn tree_sitter_groovy() -> *const ();
}

// SAFETY: tree_sitter_groovy is the entry point of grammar/parser.c, compiled by build.rs. It
// takes no arguments and returns a pointer to the grammar's static TSLanguage, which is what
// from_raw requires. The only unsafe code in groovyfmt.
const GROOVY: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_groovy) };

pub fn parse(src: &str) -> Tree {
    // one parser per thread, reused: a file is parsed two to four times
    thread_local! {
        static PARSER: std::cell::RefCell<Parser> = std::cell::RefCell::new({
            let mut p = Parser::new();
            p.set_language(&GROOVY.into())
                .expect("grammar ABI mismatch");
            p
        });
    }
    PARSER.with_borrow_mut(|p| p.parse(src, None).expect("parse cancelled"))
}

/// Visit every node in document order. Iterative, so deeply nested input can't overflow the
/// stack; `f` returns false to skip a node's children.
pub fn preorder<'a>(root: Node<'a>, mut f: impl FnMut(Node<'a>) -> bool) {
    let mut c = root.walk();
    loop {
        if f(c.node()) && c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return;
            }
        }
    }
}

/// Source text of a node.
pub fn text<'a>(n: Node, src: &'a str) -> &'a str {
    n.utf8_text(src.as_bytes()).unwrap_or("")
}

/// First ERROR or MISSING node in document order, if any.
pub fn first_error(tree: &Tree) -> Option<Node<'_>> {
    if !tree.root_node().has_error() {
        return None;
    }
    let mut c = tree.walk();
    loop {
        let n = c.node();
        if n.is_error() || n.is_missing() {
            return Some(n);
        }
        if n.has_error() && c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return None;
            }
        }
    }
}

/// A path as groovyfmt prints it (reports, JSON, SARIF, patch headers, errors): on Windows
/// with `/` between components, so output and patches are the same on every platform. A
/// name that isn't valid Unicode shows U+FFFD in place of the bad part.
pub fn display_path(p: &Path) -> String {
    let s = p.display().to_string();
    // `\` can't be part of a file name on Windows, so this changes only separators
    #[cfg(windows)]
    let s = s.replace('\\', "/");
    s
}

/// Groovy sources, Gradle build scripts, and Jenkinsfiles.
pub fn is_groovy_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    [".groovy", ".gradle", ".gvy", ".gy", ".gsh"]
        .iter()
        .any(|ext| name.ends_with(ext))
        || name == "Jenkinsfile"
}

/// Directories skipped while walking: hidden ones and common build/vendor output.
/// A directory named explicitly on the command line is always entered.
pub fn is_skipped_dir(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.starts_with('.') || matches!(name, "build" | "out" | "target" | "node_modules")
}

/// For the tests that guard against quadratic (or worse) blow-ups: how cost grows with input
/// size, measured so that a loaded machine doesn't fail them. A fixed limit in seconds did:
/// on a shared box at load 15 the linear tests took 3 to 4 times longer, in parallel with the
/// rest of the suite.
#[cfg(test)]
pub(crate) mod scaling {
    use std::time::{Duration, Instant};

    /// The CPU time this thread has used, where the OS reports it without a dependency
    /// (Linux: `/proc/thread-self/stat`), so other tests and processes don't count.
    fn thread_cpu() -> Option<Duration> {
        // Linux's: Wine makes the path answer on Windows builds too, from the host's /proc
        if !cfg!(target_os = "linux") {
            return None;
        }
        let s = std::fs::read_to_string("/proc/thread-self/stat").ok()?;
        // after the parenthesized command name comes field 3; utime and stime are 14 and 15
        let fields: Vec<&str> = s.get(s.rfind(')')? + 2..)?.split(' ').collect();
        let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
        // USER_HZ is 100 on every Linux target
        Some(Duration::from_millis(ticks * 10))
    }

    /// How long `reps` calls of `f(n)` take: this thread's CPU time where available, else
    /// wall time, which counts whatever else the machine is doing too, so the fastest of
    /// three tries.
    fn cost(f: &impl Fn(usize), n: usize, reps: usize) -> Duration {
        let run = || (0..reps).for_each(|_| f(n));
        let (cpu, wall) = (thread_cpu(), Instant::now());
        run();
        match (cpu, thread_cpu()) {
            (Some(a), Some(b)) => b.saturating_sub(a),
            _ => (0..2)
                .map(|_| {
                    let t = Instant::now();
                    run();
                    t.elapsed()
                })
                .fold(wall.elapsed(), Duration::min),
        }
    }

    /// The least the smaller measurement may cost: ten ticks of the coarsest clock used
    /// (Linux's thread CPU time counts in 10 ms), so a tick either way is noise, not a ratio.
    const MIN_COST: Duration = Duration::from_millis(100);

    /// Assert that `f` on `k` times the input costs at most `bound` times what it costs on
    /// the input: under 8x for 4x the input passes anything linear (or n log n) and fails
    /// anything quadratic (16x). The input starts at `n` and doubles until a run costs
    /// `MIN_COST`, so the comparison is between costs the clock measures well, however fast
    /// the machine, and the growing part of the cost outweighs the fixed one. A failure is
    /// measured again once, so a burst of noise in either run can't fail it.
    pub fn assert_scales(what: &str, f: impl Fn(usize), n: usize, k: usize, bound: u32) {
        assert_scales_up_to(what, f, n, usize::MAX, k, bound);
    }

    /// `assert_scales` for a workload with a hard limit: the input grows only while `k`
    /// times it stays within `max`; past that, each measurement repeats the call until the
    /// smaller one costs `MIN_COST`.
    pub fn assert_scales_up_to(
        what: &str,
        f: impl Fn(usize),
        mut n: usize,
        max: usize,
        k: usize,
        bound: u32,
    ) {
        assert!(n * k <= max, "{what}: {k} x {n} is over the limit {max}");
        while n * k * 2 <= max && cost(&f, n, 1) < MIN_COST {
            n *= 2;
        }
        let mut reps = 1;
        while cost(&f, n, reps) < MIN_COST {
            reps *= 2;
        }
        let mut seen = (Duration::ZERO, Duration::ZERO);
        for _ in 0..2 {
            let small = cost(&f, n, reps);
            let big = cost(&f, n * k, reps);
            if big <= small * bound {
                return;
            }
            seen = (small, big);
        }
        panic!(
            "{what}: {k}x the input ({n} to {}, {reps} runs each) took {:?} against {:?}, over {bound}x",
            n * k,
            seen.1,
            seen.0
        );
    }

    fn linear(n: usize) {
        std::hint::black_box((0..n as u64 * 1_000).fold(0u64, |a, b| a ^ b));
    }

    fn quadratic(n: usize) {
        let n = n as u64 * 10;
        std::hint::black_box((0..n * n).fold(0u64, |a, b| a ^ b));
    }

    #[test]
    fn linear_work_passes() {
        assert_scales("linear", linear, 1, 4, 8);
        assert_scales_up_to("linear, capped", linear, 1, 8, 4, 8);
    }

    #[test]
    #[should_panic(expected = "over 8x")]
    fn quadratic_work_fails() {
        // from a size far below the clock's resolution, however fast the machine
        assert_scales("quadratic", quadratic, 1, 4, 8);
    }

    #[test]
    #[should_panic(expected = "over 8x")]
    fn quadratic_work_at_a_hard_limit_fails() {
        // can't grow: the repeated calls still show 16x
        assert_scales_up_to("quadratic, capped", quadratic, 8, 32, 4, 8);
    }
}

#[cfg(test)]
mod tests {
    /// Every node kind and field name matched as a string in format.rs and lint.rs. A grammar
    /// update that renames one would otherwise silently turn a lint rule into a no-op.
    #[test]
    fn grammar_has_every_kind_and_field_we_match() {
        let lang: tree_sitter::Language = super::GROOVY.into();
        let named = [
            "source_file",
            "identifier",
            "block",
            "class_body",
            "enum_body",
            "closure",
            "method_invocation",
            "command_chain",
            "string_fragment",
            "switch_block",
            "switch_case",
            "labeled_statement",
            "import_declaration",
            "if_statement",
            "while_statement",
            "for_statement",
            "for_in_statement",
            "try_statement",
            "catch_clause",
            "finally_clause",
            "map_literal",
            "map_entry",
            "binary_expression",
            "assignment_expression",
            "parenthesized_expression",
            "string_literal",
            "number_literal",
            "boolean_literal",
            "null_literal",
            "line_comment",
            "block_comment",
            "groovydoc_comment",
            "annotation",
            "gstring_brace_interpolation",
            "gstring_dollar_interpolation",
            // lint/jenkins.rs, lint/gradle.rs, lint/spock.rs
            "pipeline_statement",
            "expression_statement",
            "closure_parameters",
            "named_argument",
            "local_variable_declaration",
            "for_in_statement",
            "do_while_statement",
            "switch_expression",
            "assignment_expression",
            "package_declaration",
            "class_declaration",
            "superclass",
            "method_declaration",
            "quoted_identifier",
            // spacing.rs
            "expression_statement",
            "named_argument",
            "argument_list",
            "formal_parameters",
            "formal_parameter",
            "closure_parameters",
            "type_arguments",
            "type_parameters",
            "array_type",
            "dimension",
            "dimension_expression",
            "method_reference_expression",
            "unary_expression",
            "update_expression",
            "unary_update_expression",
            "spread_arguments",
            "ternary_expression",
            "elvis_expression",
            "cast_expression",
            "parenthesized_type_cast",
            "object_creation_expression",
            "lambda_expression",
            "variable_declarator",
            "local_variable_declaration",
            "field_declaration",
            "multi_assignment_declaration",
            "multi_assignment_expression",
            "switch_default",
            "switch_arrow_case",
            "switch_expression",
            "quoted_identifier",
            "power_expression",
            "membership_expression",
            "instanceof_expression",
            "spaceship_expression",
            "regex_find_expression",
            "regex_match_expression",
            "identity_expression",
            "logical_implication_expression",
            "return_statement",
            "list_literal",
            "do_while_statement",
            "synchronized_statement",
            // lint/basic.rs, lint/imports.rs, lint/unused.rs
            "switch_default",
            "static_initializer",
            "instance_initializer",
            "cast_expression",
            "ternary_expression",
            "elvis_expression",
            "assert_statement",
            "throw_statement",
            "return_statement",
            "break_statement",
            "continue_statement",
            "unary_expression",
            "field_access",
            "subscript_expression",
            "field_declaration",
            "variable_declarator",
            "formal_parameters",
            "formal_parameter",
            "object_creation_expression",
            "list_literal",
            "constructor_declaration",
            "lambda_expression",
            "spread_arguments",
            "spread_map_entry",
            "interface_declaration",
            "enum_declaration",
            "trait_declaration",
            "record_declaration",
            "annotation_type_declaration",
        ];
        for k in named {
            assert_ne!(lang.id_for_node_kind(k, true), 0, "missing node kind {k}");
        }
        for k in [
            ";",
            "=",
            "==",
            "{",
            "}",
            "(",
            ")",
            "[",
            "]",
            ",",
            ":",
            "?",
            "->",
            "::",
            "<",
            ">",
            "|",
            "||",
            "else",
            "catch",
            "finally",
            "while",
            "if",
            "for",
            "switch",
            "synchronized",
            "<<",
            "&&",
            "!=",
            "%",
            "private",
            "static",
        ] {
            assert_ne!(lang.id_for_node_kind(k, false), 0, "missing token {k}");
        }
        let fields = [
            "name",
            "alias",
            "body",
            "parameter",
            "condition",
            "consequence",
            "alternative",
            "key",
            "value",
            "operator",
            "left",
            "right",
            "label",
            "receiver",
            "argument",
            "function",
            "arguments",
            "closure",
            "statement",
            "base",
            "type",
            "return_type",
            "parameters",
            "object",
            "field",
            "operand",
        ];
        for f in fields {
            assert!(lang.field_id_for_name(f).is_some(), "missing field {f}");
        }
    }
}
