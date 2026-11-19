// Hand-written external scanner for tree-sitter-groovy.
//
// Handles §6.2 slashy-string-start and §6.6 line/block/groovydoc
// comments. Both start with `/`; combining the dispatch keeps the
// in-grammar lexer from partially consuming the leading `/`.
//
// Stateless — `create` returns NULL and the runtime calls
// `destroy` / `serialize` / `deserialize` as no-ops. When a future
// branch needs persistent state, introduce a struct then.

#include "tree_sitter/parser.h"

#include <string.h>

enum TokenType {
    SLASHY_STRING_START,
    LINE_COMMENT,
    BLOCK_COMMENT,
    GROOVYDOC_COMMENT,
    COMMAND_NAMED_KEY,
    SUBSCRIPT_OPEN,
    COMMAND_LITERAL_START,
    COMMAND_CLOSURE_START,
    CALL_OPEN,
    CAST_VALUE_START,
    STRING_BODY,
    RECORD_KEYWORD,
    COMMAND_VALUE_START,
    LINE_BREAK,
    COMMAND_NAME_ARGUMENT,
    VAR_KEYWORD,
    PIPELINE_KEYWORD,
    TRAIT_KEYWORD,
    TYPE_ARGUMENTS_OPEN,
    LESS_THAN,
    YIELD_KEYWORD,
    TERNARY_QUESTION,
    EXPRESSION_COLON,
    COMMAND_NAME_START,
    CASE_ARROW,
    CLOSURE_COMMAND_ARGUMENT,
    COMMAND_LAMBDA_OPEN,
    CLOSURE_DECLARATION_START,
    NOT_IN_OPEN,
    NEW_CLOSURE_START,
    SEALED_KEYWORD,
    COMMAND_KEYWORD_ARGUMENT,
    DIMENSION_ANNOTATION_START,
    COMMAND_OPERAND,
    ERROR_SENTINEL,
};

static inline bool is_ident_start(int32_t c) {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_'
        || c == '$' || c > 127;
}

static inline bool is_ident_part(int32_t c) {
    return is_ident_start(c) || (c >= '0' && c <= '9');
}

// SPECIFICATION.md §5.5 — the key of a named argument in a
// parenthesis-free call (`exclude group: 'a'`). Matches an identifier
// on the current line that is followed by `:` (not `::`), consuming
// only the identifier. A newline before the key means a new statement
// (a Spock `where:` label, say), so no match.
//
// The same identifier is `_command_name_argument`, the one argument of a
// command call on a dotted receiver (`a.b key`), when the statement ends
// after it: a line break, `;`, `}`, a comment or the end of input.
static bool finish_command_named_key(TSLexer *lexer, const bool *valid_symbols,
                                     const char *word, unsigned len);
static unsigned read_word(TSLexer *lexer, char *word, unsigned size);
static bool else_follows(TSLexer *lexer);

static bool scan_command_named_key(TSLexer *lexer, const bool *valid_symbols) {
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        lexer->advance(lexer, true);
    }
    if (!is_ident_start(lexer->lookahead)) {
        return false;
    }
    // Keep the first few chars to reject the operators `as`, `in` and
    // `instanceof`, which are not arguments.
    char word[12];
    unsigned len = read_word(lexer, word, sizeof(word));
    return finish_command_named_key(lexer, valid_symbols, word, len);
}

// Reads an identifier into `word` (its first `size` chars) and returns its
// length.
static unsigned read_word(TSLexer *lexer, char *word, unsigned size) {
    unsigned len = 0;
    while (is_ident_part(lexer->lookahead)) {
        if (len < size) {
            word[len] = (char)lexer->lookahead;
        }
        len++;
        lexer->advance(lexer, false);
    }
    return len;
}

// Whether a name may be a command call's one argument: not one of the
// operators `as`, `in` and `instanceof`, nor `case` or `default`.
static bool is_name_argument(const char *word, unsigned len) {
    return !((len == 2 && (memcmp(word, "as", 2) == 0 || memcmp(word, "in", 2) == 0))
        || (len == 10 && memcmp(word, "instanceof", 10) == 0)
        || (len == 4 && memcmp(word, "case", 4) == 0)
        || (len == 7 && memcmp(word, "default", 7) == 0));
}

static inline bool is_operator_char(int32_t c) {
    return c == '+' || c == '-' || c == '*' || c == '%' || c == '<' || c == '>' || c == '&'
        || c == '|' || c == '^' || c == '=' || c == '!';
}

// `_command_operand`, with the lexer at an operator character after the name:
// a binary operator (none that is a node of its own, an assignment or `/`,
// which may start a comment), the rest of the line, and an `else` starting
// the next one.
static bool finish_command_operand(TSLexer *lexer) {
    static const char *const ops[] = {
        "+", "-", "*", "%", "<<", ">>", ">>>", "<", "<=", ">", ">=",
        "==", "!=", "&", "^", "|", "&&", "||",
    };
    char op[5];
    unsigned len = 0;
    while (is_operator_char(lexer->lookahead)) {
        if (len == sizeof(op) - 1) {
            return false;
        }
        op[len++] = (char)lexer->lookahead;
        lexer->advance(lexer, false);
    }
    op[len] = '\0';
    bool known = false;
    for (unsigned i = 0; i < sizeof(ops) / sizeof(ops[0]); i++) {
        known |= strcmp(op, ops[i]) == 0;
    }
    // `==~` and `=~` are regex matches, not `== ~x`.
    if (!known || lexer->lookahead == '~') {
        return false;
    }
    while (!lexer->eof(lexer) && lexer->lookahead != '\n') {
        lexer->advance(lexer, false);
    }
    if (!else_follows(lexer)) {
        return false;
    }
    lexer->result_symbol = COMMAND_OPERAND;
    return true;
}

// The rest of `scan_command_named_key`, once the identifier is read.
static bool finish_command_named_key(TSLexer *lexer, const bool *valid_symbols,
                                     const char *word, unsigned len) {
    // `case` and `default` may be keys too, since a switch label can't follow
    // an argument on its line: `input name: 'x', default: false`.
    lexer->mark_end(lexer);
    // A closure's whole body, `{ x -> println x }`: the name and then `}`.
    if (valid_symbols[CLOSURE_COMMAND_ARGUMENT] && is_name_argument(word, len)
        && !(len == 4 && memcmp(word, "else", 4) == 0)) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, false);
        }
        if (lexer->lookahead == '}') {
            lexer->result_symbol = CLOSURE_COMMAND_ARGUMENT;
            return true;
        }
    }
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        lexer->advance(lexer, false);
    }
    int32_t n = lexer->lookahead;
    // `if (c) o.f a - b` then `else`: the name a binary operator follows, as
    // the first operand of a braceless if body's one command argument. None of
    // the tokens below starts with an operator, so a miss returns false.
    if (valid_symbols[COMMAND_OPERAND] && is_name_argument(word, len)
        && !(len == 4 && memcmp(word, "else", 4) == 0) && is_operator_char(n)) {
        return finish_command_operand(lexer);
    }
    if (n == ':' && valid_symbols[COMMAND_NAMED_KEY]) {
        lexer->advance(lexer, false);
        if (lexer->lookahead == ':') {
            return false;
        }
        lexer->result_symbol = COMMAND_NAMED_KEY;
        return true;
    }
    // `println as`, `println in`: a variable named `as` or `in` (Groovy 3 and
    // 4) as a bare receiver's one argument, when the statement ends after it.
    bool keyword = len == 2 && (memcmp(word, "as", 2) == 0 || memcmp(word, "in", 2) == 0);
    if (!(valid_symbols[COMMAND_NAME_ARGUMENT] && is_name_argument(word, len))
        && !(valid_symbols[COMMAND_KEYWORD_ARGUMENT] && keyword)) {
        return false;
    }
    bool ends = lexer->eof(lexer) || n == '\n' || n == '\r' || n == ';' || n == '}';
    if (n == '/') {
        lexer->advance(lexer, false);
        ends = lexer->lookahead == '/' || lexer->lookahead == '*';
    }
    // `if (a) warn line else info line`: `else` ends a braceless if body
    // on its line, since it is reserved and continues no expression.
    if (n == 'e') {
        char next[5];
        ends = read_word(lexer, next, sizeof(next)) == 4 && memcmp(next, "else", 4) == 0;
    }
    if (!ends) {
        return false;
    }
    lexer->result_symbol = keyword ? COMMAND_KEYWORD_ARGUMENT : COMMAND_NAME_ARGUMENT;
    return true;
}

// SPECIFICATION.md §5.8 — a closure's or block's whole body that is a
// declaration with no value, `{ Foo bar }`, `{ x -> java.util.List l }`:
// a dotted name whose last part starts with a capital letter, then a name
// and `}` on its line. Groovy reads that as a declaration where `{ foo bar }`
// and `{ Foo.bar baz }` are command calls (`_closure_command_argument`), and
// `{ _Foo bar }` and `{ this.Foo bar }` too. Called with the lexer just past
// the first part, which was `this` or `super` when `self`, and whose first
// char was `first`; the token is the zero-width `_closure_declaration_start`,
// so mark_end was called before it.
// ponytail: Latin-1, Greek and Cyrillic capitals only (Groovy asks
// Character.isUpperCase); a table if a type in another script shows up.
static bool is_capital(int32_t c) {
    return (c >= 'A' && c <= 'Z') || (c >= 0xC0 && c <= 0xDE && c != 0xD7)
        || (c >= 0x391 && c <= 0x3A9 && c != 0x3A2) || (c >= 0x400 && c <= 0x42F);
}

static bool finish_closure_declaration(TSLexer *lexer, int32_t first, bool self) {
    if (self) {
        return false;
    }
    bool capital = is_capital(first);
    while (lexer->lookahead == '.') {
        lexer->advance(lexer, false);
        if (!is_ident_start(lexer->lookahead)) {
            return false;
        }
        capital = is_capital(lexer->lookahead);
        while (is_ident_part(lexer->lookahead)) {
            lexer->advance(lexer, false);
        }
    }
    if (!capital || (lexer->lookahead != ' ' && lexer->lookahead != '\t')) {
        return false;
    }
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        lexer->advance(lexer, false);
    }
    if (!is_ident_start(lexer->lookahead)) {
        return false;
    }
    char word[12];
    unsigned len = read_word(lexer, word, sizeof(word));
    if (!is_name_argument(word, len) || (len == 4 && memcmp(word, "else", 4) == 0)) {
        return false;
    }
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        lexer->advance(lexer, false);
    }
    if (lexer->lookahead != '}') {
        return false;
    }
    lexer->result_symbol = CLOSURE_DECLARATION_START;
    return true;
}

static inline void advance(TSLexer *lexer) {
    lexer->advance(lexer, false);
}

// A byte order mark counts too: a file may start with two of them.
static inline bool is_whitespace(int32_t c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == 0xFEFF;
}

// The skippers below look ahead past a whole `{…}` or `(…)` to choose a
// reading, and a scan that fails starts again at the next `{` or `(`: on
// unbalanced input that is quadratic, and a GString's `${…}` nests by
// recursion. A budget of steps bounds both; a scan that runs out answers
// false, the caller's default reading.
// ponytail: a fixed budget; a `pipeline { … }.with { }` whose body runs past it
// reads as the pipeline block, as an unbalanced one does.
#define SKIP_BUDGET 4000

// Whether the scan may take one more step: not at the end of input and
// budget left.
static inline bool skip_step(TSLexer *lexer, unsigned *budget) {
    if (lexer->eof(lexer) || *budget == 0) {
        return false;
    }
    --*budget;
    return true;
}

// Skips whitespace; true when a line ended in it.
// Advances past a balanced `{…}`, the lookahead being its `{`; false at the
// end of input. Strings, GString `${…}` and comments are skipped, so their
// braces don't count. Slashy strings are not recognized: a brace in one may
// end the scan early, and the caller then keeps its default reading, as it
// does when the scan runs out of its step budget (see skip_step).
static bool skip_braces(TSLexer *lexer, unsigned *budget) {
    unsigned depth = 0;
    while (skip_step(lexer, budget)) {
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '{') {
            depth++;
        } else if (c == '}') {
            if (--depth == 0) {
                return true;
            }
        } else if (c == '/' && lexer->lookahead == '/') {
            while (skip_step(lexer, budget) && lexer->lookahead != '\n') {
                advance(lexer);
            }
        } else if (c == '/' && lexer->lookahead == '*') {
            advance(lexer);
            int32_t prev = 0;
            while (skip_step(lexer, budget) && !(prev == '*' && lexer->lookahead == '/')) {
                prev = lexer->lookahead;
                advance(lexer);
            }
            advance(lexer);
        } else if (c == '\'' || c == '"') {
            bool triple = false;
            if (lexer->lookahead == c) {
                advance(lexer);
                if (lexer->lookahead != c) {
                    continue; // empty string
                }
                advance(lexer);
                triple = true;
            }
            unsigned run = 0;
            while (skip_step(lexer, budget)) {
                int32_t d = lexer->lookahead;
                if (d == '\\') {
                    advance(lexer);
                    if (lexer->lookahead == '\r') {
                        advance(lexer); // a CRLF line continuation
                    }
                    advance(lexer);
                    run = 0;
                    continue;
                }
                if (!triple && d == '\n') {
                    break;
                }
                if (c == '"' && d == '$') {
                    advance(lexer);
                    if (lexer->lookahead == '{' && !skip_braces(lexer, budget)) {
                        return false;
                    }
                    run = 0;
                    continue;
                }
                advance(lexer);
                if (d == c) {
                    run++;
                    if (!triple || run == 3) {
                        break;
                    }
                } else {
                    run = 0;
                }
            }
        }
    }
    return false;
}

// Advances to just past the `)` that closes a `(` already read; false at the
// end of input or of the step budget. Quoted strings are skipped, so their
// parentheses don't count.
static bool skip_parens(TSLexer *lexer) {
    unsigned depth = 1;
    unsigned budget = SKIP_BUDGET;
    while (skip_step(lexer, &budget)) {
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '(') {
            depth++;
        } else if (c == ')') {
            if (--depth == 0) {
                return true;
            }
        } else if (c == '\'' || c == '"') {
            while (skip_step(lexer, &budget) && lexer->lookahead != c && lexer->lookahead != '\n') {
                if (lexer->lookahead == '\\') {
                    advance(lexer);
                    if (lexer->lookahead == '\r') {
                        advance(lexer); // a CRLF line continuation
                    }
                }
                advance(lexer);
            }
            advance(lexer);
        }
    }
    return false;
}

// Advances past the rest of a `(…)` that holds only what a lambda's parameter
// list can: names, whitespace, `,`, `.`, `<>`, `[]`, `?` and `@`. False at any
// other character.
static bool skip_parameter_parens(TSLexer *lexer) {
    while (!lexer->eof(lexer)) {
        int32_t c = lexer->lookahead;
        if (c == ')') {
            advance(lexer);
            return true;
        }
        if (!(is_ident_part(c) || is_whitespace(c) || c == ',' || c == '.' || c == '<'
              || c == '>' || c == '[' || c == ']' || c == '?' || c == '@')) {
            return false;
        }
        advance(lexer);
    }
    return false;
}

// Whether `else` comes next, past whitespace, line breaks and comments.
static bool else_follows(TSLexer *lexer) {
    for (;;) {
        while (is_whitespace(lexer->lookahead)) {
            advance(lexer);
        }
        if (lexer->lookahead != '/') {
            break;
        }
        advance(lexer);
        if (lexer->lookahead == '/') {
            while (!lexer->eof(lexer) && lexer->lookahead != '\n') {
                advance(lexer);
            }
        } else if (lexer->lookahead == '*') {
            advance(lexer);
            for (;;) {
                if (lexer->eof(lexer)) {
                    return false;
                }
                int32_t c = lexer->lookahead;
                advance(lexer);
                if (c == '*' && lexer->lookahead == '/') {
                    advance(lexer);
                    break;
                }
            }
        } else {
            return false;
        }
    }
    char word[5];
    return read_word(lexer, word, sizeof(word)) == 4 && memcmp(word, "else", 4) == 0;
}

static bool skip_whitespace(TSLexer *lexer) {
    bool newline = false;
    while (is_whitespace(lexer->lookahead)) {
        newline |= lexer->lookahead == '\n' || lexer->lookahead == '\r';
        lexer->advance(lexer, true);
    }
    return newline;
}

// Skips whitespace and `\` line continuations; true when a line ended in
// the whitespace (a continued line doesn't count). Sets *stray when a `\`
// that continues no line was skipped: no token may start after it.
static bool skip_whitespace_and_continuations(TSLexer *lexer, bool *stray) {
    bool newline = false;
    for (;;) {
        newline |= skip_whitespace(lexer);
        if (lexer->lookahead != '\\') {
            return newline;
        }
        lexer->advance(lexer, true);
        if (lexer->lookahead == '\r') {
            lexer->advance(lexer, true);
        }
        if (lexer->lookahead != '\n') {
            *stray = true;
            return newline;
        }
        lexer->advance(lexer, true);
    }
}

// At a `[`: true when a balanced `[…]` (brackets, braces and parentheses
// nested; quoted strings skipped) is followed on its line by `:` but not `::`.
// A call or property chain may come between: `a?[b].c():d`.
static bool bracket_then_colon(TSLexer *lexer) {
    unsigned depth = 0;
    bool chain = false;
    for (unsigned steps = 0; steps < 4000 && !lexer->eof(lexer); steps++) {
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '[' || c == '(' || c == '{') {
            depth++;
        } else if (c == ']' || c == ')' || c == '}') {
            if (--depth == 0) {
                if (c != ']' && !chain) {
                    return false;
                }
                chain = true;
                for (;;) {
                    if (lexer->lookahead == '?' || lexer->lookahead == '*') {
                        advance(lexer);
                        if (lexer->lookahead != '.') {
                            return false;
                        }
                    }
                    if (lexer->lookahead != '.') {
                        break;
                    }
                    advance(lexer);
                    if (!is_ident_start(lexer->lookahead)) {
                        return false;
                    }
                    while (is_ident_part(lexer->lookahead)) {
                        advance(lexer);
                    }
                    // a closure argument: `.findAll { it }`
                    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                        advance(lexer);
                    }
                }
                if (lexer->lookahead == '(' || lexer->lookahead == '[' || lexer->lookahead == '{') {
                    continue;
                }
                // the `:` may start the next line: `c` then `?[x]` then `:y`
                while (is_whitespace(lexer->lookahead)) {
                    advance(lexer);
                }
                if (lexer->lookahead != ':') {
                    return false;
                }
                advance(lexer);
                return lexer->lookahead != ':';
            }
        } else if (c == '\'' || c == '"') {
            while (!lexer->eof(lexer) && lexer->lookahead != c && steps++ < 4000) {
                if (lexer->lookahead == '\\') {
                    advance(lexer);
                }
                advance(lexer);
            }
            advance(lexer);
        }
    }
    return false;
}

// At a `<` right after a name: true when a balanced, type-shaped `<…>` with
// no space just inside its outer `<` and `>` (names, dots, commas, `?`, `[]`
// and nested `<…>`) ends its argument: `,`, `;`, `}` or the line's end follow.
// So does `.` and a name: `tag List<String>.class`. No comparison ends in `>`
// there: `tag List<String>` is `tag(List<String>)`.
static bool type_arguments_end_argument(TSLexer *lexer) {
    advance(lexer);
    if (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        return false;
    }
    unsigned depth = 1;
    bool prev_space = false;
    for (unsigned steps = 0; depth > 0; steps++) {
        int32_t c = lexer->lookahead;
        if (steps > 400 || lexer->eof(lexer)) {
            return false;
        }
        bool space = c == ' ' || c == '\t';
        if (is_ident_start(c)) {
            while (is_ident_part(lexer->lookahead)) {
                advance(lexer);
            }
            prev_space = false;
            continue;
        }
        if (c == '<') {
            depth++;
        } else if (c == '>') {
            if (--depth == 0 && prev_space) {
                return false;
            }
        } else if (c == '[') {
            advance(lexer);
            if (lexer->lookahead != ']') {
                return false;
            }
        } else if (!space && c != '.' && c != ',' && c != '?') {
            return false;
        }
        advance(lexer);
        prev_space = space;
    }
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        advance(lexer);
    }
    int32_t c = lexer->lookahead;
    if (c == '.') {
        advance(lexer);
        return is_ident_start(lexer->lookahead);
    }
    return c == ',' || c == ';' || c == '}' || c == '\n' || c == '\r' || lexer->eof(lexer);
}

void *tree_sitter_groovy_external_scanner_create(void) {
    return NULL;
}

void tree_sitter_groovy_external_scanner_destroy(void *payload) {
    (void)payload;
}

unsigned tree_sitter_groovy_external_scanner_serialize(
    void *payload,
    char *buffer
) {
    (void)payload;
    (void)buffer;
    return 0;
}

void tree_sitter_groovy_external_scanner_deserialize(
    void *payload,
    const char *buffer,
    unsigned length
) {
    (void)payload;
    (void)buffer;
    (void)length;
}

bool tree_sitter_groovy_external_scanner_scan(
    void *payload,
    TSLexer *lexer,
    const bool *valid_symbols
) {
    (void)payload;
    // whether the whitespace skipped before the token held a line end
    bool newline = false;

    // SPECIFICATION.md §5.7 — inside a string body `//` and `/*` are text,
    // never a comment; the grammar's text tokens take them.
    if (valid_symbols[STRING_BODY] && !valid_symbols[ERROR_SENTINEL]) {
        return false;
    }

    // SPECIFICATION.md §5.14 — the `<` of a generic type used as a value,
    // right after its name: `Mock(Supplier<Long>)`, `List<X>.class`. Only
    // when a balanced `<…>` of names, dots, commas, `?`, `[]` and nested
    // `<…>` follows on the line, and after its `>` comes `)`, `,`, `]`,
    // `::`, or `.` and a name (also on the next line). `i<n`, `a<b && c>d`
    // and `f(a<b, c>d)` don't match, so their `<` stays an operator.
    // After a space the `<` must also be followed, past its `>`, by a name
    // and `=`, `;`, `(` or the line's end, or by a `(`: a declaration,
    // `Map <K, V> m = [:]`, or `new TreeSet <String>()`.
    bool spaced = false;
    if (valid_symbols[TYPE_ARGUMENTS_OPEN] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
            spaced = true;
        }
    }
    if (valid_symbols[TYPE_ARGUMENTS_OPEN] && !valid_symbols[ERROR_SENTINEL]
        && lexer->lookahead == '<') {
        advance(lexer);
        lexer->mark_end(lexer);
        // A comparison's `<` when no balanced, type-shaped `<…>` follows.
        // Not `<=`, `<<`, `<..` or `<=>`, which the grammar lexes.
        bool less_than = valid_symbols[LESS_THAN] && lexer->lookahead != '='
            && lexer->lookahead != '<' && lexer->lookahead != '.';
        unsigned depth = 1;
        bool after_name = false;
        // No space just inside the outer `<` and `>`, as in `<V>` but not in
        // the comparisons `x < y, z > w`.
        bool tight = lexer->lookahead != ' ' && lexer->lookahead != '\t';
        bool prev_space = false;
        // A line may break after a `,` or the `<` (`Map<String,` then `Integer> m = x`),
        // but only a declaration then follows the `>`.
        bool after_comma = true;
        bool multiline = false;
        for (unsigned steps = 0; depth > 0; steps++) {
            int32_t c = lexer->lookahead;
            if (steps > 400 || lexer->eof(lexer)) {
                return false;
            }
            if (c == '\n' || c == '\r') {
                if (!after_comma || spaced) {
                    goto not_type;
                }
                multiline = true;
                advance(lexer);
                prev_space = true;
                continue;
            }
            if (c != ' ' && c != '\t') {
                after_comma = c == ',' || c == '<';
            }
            if (c == ' ' || c == '\t') {
                advance(lexer);
                prev_space = true;
                continue;
            } else if (is_ident_start(c)) {
                while (is_ident_part(lexer->lookahead)) {
                    advance(lexer);
                }
                after_name = true;
                prev_space = false;
                continue;
            } else if (c == '.') {
                advance(lexer);
                if (!after_name || !is_ident_start(lexer->lookahead)) {
                    goto not_type;
                }
            } else if (c == ',' || c == '?' || c == '@') {
                advance(lexer);
            } else if (c == '/') {
                // A block comment on the line counts as a space: `<? /*V*/>`.
                advance(lexer);
                if (lexer->lookahead != '*') {
                    goto not_type;
                }
                advance(lexer);
                for (;;) {
                    if (lexer->eof(lexer) || lexer->lookahead == '\n'
                        || lexer->lookahead == '\r') {
                        goto not_type;
                    }
                    int32_t star = lexer->lookahead;
                    advance(lexer);
                    if (star == '*' && lexer->lookahead == '/') {
                        advance(lexer);
                        break;
                    }
                }
                prev_space = true;
                continue;
            } else if (c == '[') {
                advance(lexer);
                if (lexer->lookahead != ']') {
                    goto not_type;
                }
                advance(lexer);
            } else if (c == '<') {
                advance(lexer);
                depth++;
            } else if (c == '>') {
                advance(lexer);
                depth--;
                if (depth == 0 && prev_space) {
                    tight = false;
                }
            } else {
                goto not_type;
            }
            after_name = false;
            prev_space = false;
        }
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            advance(lexer);
        }
        int32_t c = lexer->lookahead;
        bool ok = c == ')' || c == ',' || c == ']';
        if (multiline) {
            ok = false;
            if (is_ident_start(c)) {
                while (is_ident_part(lexer->lookahead)) {
                    advance(lexer);
                }
                while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                    advance(lexer);
                }
                c = lexer->lookahead;
                ok = c == ';' || c == '\n' || c == '\r' || lexer->eof(lexer);
                if (c == '=') {
                    advance(lexer);
                    ok = lexer->lookahead != '=' && lexer->lookahead != '~';
                }
            }
        } else if (spaced) {
            // A cast `(List <T>) x`, or a call `new TreeSet <String>()`.
            ok = c == '(' || (c == ')' && tight);
            if (is_ident_start(c)) {
                while (is_ident_part(lexer->lookahead)) {
                    advance(lexer);
                }
                while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                    advance(lexer);
                }
                c = lexer->lookahead;
                // `Closure <V> value)`, `Map <K, V> m, …`: a parameter.
                ok = c == ';' || c == '(' || ((c == ')' || c == ',') && tight)
                    || c == '\n' || c == '\r' || lexer->eof(lexer);
                if (c == '=') {
                    advance(lexer);
                    ok = lexer->lookahead != '=' && lexer->lookahead != '~';
                } else if ((c == '-' && !valid_symbols[CASE_ARROW])
                    || (c == ':' && !valid_symbols[EXPRESSION_COLON])) {
                    // A closure parameter `{ Map <K, V> e -> }` or a for
                    // variable `for (Map <K, V> e : xs)`. Outside a case
                    // label (`case a < b, c > d:`) no comparison ends in a
                    // name and `->` or `:`.
                    advance(lexer);
                    ok = c == '-' ? lexer->lookahead == '>' : lexer->lookahead != ':';
                } else if (c == 'i') {
                    // `for (Map <K, V> e in xs)`.
                    advance(lexer);
                    if (lexer->lookahead == 'n') {
                        advance(lexer);
                        ok = lexer->lookahead == ' ' || lexer->lookahead == '\t';
                    }
                }
            }
            // Where no comparison may stand (a parameter's type, a type
            // argument) a balanced `<…>` is type arguments whatever follows:
            // `f(Map < K, List < V > > m)`. Elsewhere `f(x < y, z > w)` is
            // two comparisons.
            ok = ok || !valid_symbols[LESS_THAN];
        } else if (c == ':') {
            // `::`, or a case label `case List<String>:`; no comparison
            // ends in `>:`.
            ok = true;
        } else if (c == '-') {
            // `case List<String> -> …`: `>` then `->` ends no comparison.
            advance(lexer);
            ok = lexer->lookahead == '>';
        } else if (c == ';' || c == '}') {
            ok = tight;
        } else if (c == '.' || c == '\n' || c == '\r' || lexer->eof(lexer)) {
            // A type at the end of its line is a command call's last
            // argument, `tag List<String>` (Groovy 3 and 4), unless a `.`
            // on the next line continues it. No comparison ends in `>`.
            ok = c != '.' && tight;
            while (is_whitespace(lexer->lookahead)) {
                advance(lexer);
            }
            if (lexer->lookahead == '.') {
                advance(lexer);
                ok = is_ident_start(lexer->lookahead);
            }
        }
        if (!ok) {
            return false;
        }
        lexer->result_symbol = TYPE_ARGUMENTS_OPEN;
        return true;
    not_type:
        if (!less_than) {
            return false;
        }
        lexer->result_symbol = LESS_THAN;
        return true;
    }

    // SPECIFICATION.md §4 — the `->` after a switch case's labels, where
    // the parser expects one. A name right before it then ends the label
    // instead of starting a lambda: `case a < b -> 3`, `case FOO -> 1`.
    if (valid_symbols[CASE_ARROW] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
            spaced = true;
        }
        if (lexer->lookahead == '-') {
            advance(lexer);
            if (lexer->lookahead != '>') {
                return false;
            }
            advance(lexer);
            lexer->result_symbol = CASE_ARROW;
            return true;
        }
    }

    // SPECIFICATION.md §3.2 — the `?` of a ternary whose first branch is a
    // list or map right after it, `c?[x: 1]:[x: 2]`, as Groovy 2.4 and 3 read
    // it (Groovy 4 reads `?[` as a safe index and rejects the `:`). Only when
    // the balanced `[…]` is followed by a single `:` and no rule around the
    // expression expects a `:` (a ternary's first branch, `case`, `assert`, a
    // map key), so `c ? a?[i] : b` and `case a?[i]:` keep the safe index.
    if (valid_symbols[TERNARY_QUESTION] && !valid_symbols[EXPRESSION_COLON]
        && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
            spaced = true;
        }
        if (lexer->lookahead == '?') {
            advance(lexer);
            lexer->mark_end(lexer);
            if (lexer->lookahead == '[' && bracket_then_colon(lexer)) {
                lexer->result_symbol = TERNARY_QUESTION;
                return true;
            }
            return false;
        }
    }

    // SPECIFICATION.md §5.15 — zero-width, after a cast's `)`: the value
    // starts on the same line with a name (not `in`, `as` or `instanceof`,
    // which continue a parenthesized expression), a string or a number.
    if (valid_symbols[CAST_VALUE_START] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
            spaced = true;
        }
        int32_t c = lexer->lookahead;
        lexer->mark_end(lexer);
        if (c == '\'' || c == '"' || (c >= '0' && c <= '9')) {
            lexer->result_symbol = CAST_VALUE_START;
            return true;
        }
        if (is_ident_start(c)) {
            char word[12];
            unsigned len = 0;
            while (is_ident_part(lexer->lookahead)) {
                if (len < sizeof(word)) {
                    word[len] = (char)lexer->lookahead;
                }
                len++;
                lexer->advance(lexer, false);
            }
            // ... unless a dot follows, as in a variable named `in`: `(T) in.x`,
            // or a `[` follows `as`, which no type starts: `(T) as[0]`.
            if (((len == 2 && (memcmp(word, "in", 2) == 0 || memcmp(word, "as", 2) == 0))
                 || (len == 10 && memcmp(word, "instanceof", 10) == 0))
                && lexer->lookahead != '.'
                && !(len == 2 && memcmp(word, "as", 2) == 0 && lexer->lookahead == '[')) {
                return false;
            }
            lexer->result_symbol = CAST_VALUE_START;
            return true;
        }
        // At a line break the `_line_break` branch below decides.
        if (c != '\n' && c != '\r') {
            return false;
        }
    }

    // SPECIFICATION.md §4 — zero-width, before an array dimension's annotations:
    // `String @A []`, `new int @A [1] @B []`. Only when the annotations, each a
    // name with optional `(…)`, and then `[` follow on the line; `new int[0]`
    // then `@Override` on the next line is a member's annotation.
    if (valid_symbols[DIMENSION_ANNOTATION_START] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
            spaced = true;
        }
        if (lexer->lookahead == '@') {
            lexer->mark_end(lexer);
            while (lexer->lookahead == '@') {
                advance(lexer);
                while (is_ident_part(lexer->lookahead) || lexer->lookahead == '.') {
                    advance(lexer);
                }
                while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                    advance(lexer);
                }
                if (lexer->lookahead == '(') {
                    advance(lexer);
                    if (!skip_parens(lexer)) {
                        return false;
                    }
                    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                        advance(lexer);
                    }
                }
            }
            if (lexer->lookahead != '[') {
                return false;
            }
            lexer->result_symbol = DIMENSION_ANNOTATION_START;
            return true;
        }
    }

    // SPECIFICATION.md §5.10 — a `[` on the current line continues the
    // expression (subscript or array type). Only spaces and tabs are
    // skipped; at a newline we fall through, and the in-grammar `[` starts
    // a list or map literal instead.
    if (valid_symbols[SUBSCRIPT_OPEN] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
            spaced = true;
        }
        if (lexer->lookahead == '[') {
            lexer->advance(lexer, false);
            lexer->result_symbol = SUBSCRIPT_OPEN;
            return true;
        }
    }

    // SPECIFICATION.md §5.5 — the `(` of an argument list must be on the
    // line of the callee. At a newline we fall through, and the in-grammar
    // `(` starts a new statement instead.
    if (valid_symbols[CALL_OPEN] && !valid_symbols[ERROR_SENTINEL]) {
        // `spaced` also counts the spaces skipped by the branches above.
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            spaced = true;
            lexer->advance(lexer, true);
        }
        if (lexer->lookahead == '(') {
            lexer->advance(lexer, false);
            lexer->result_symbol = CALL_OPEN;
            // `foo (a) -> a` and `foo.bar(String s) -> s` pass a lambda (Groovy 3+);
            // `foo (a)` is a call. Without the space only a `(…)` that could be a
            // parameter list is looked through, so a call's arguments cost no scan
            // past their first other character.
            if (valid_symbols[COMMAND_LAMBDA_OPEN]) {
                lexer->mark_end(lexer);
                if (spaced ? skip_parens(lexer) : skip_parameter_parens(lexer)) {
                    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                        advance(lexer);
                    }
                    if (lexer->lookahead == '-') {
                        advance(lexer);
                        if (lexer->lookahead == '>') {
                            lexer->result_symbol = COMMAND_LAMBDA_OPEN;
                        }
                    }
                }
            }
            return true;
        }
    }

    // SPECIFICATION.md §5.5 — a zero-width marker before a command call's
    // first argument: matches only when a string or number starts on the
    // current line, so `a.b` followed by `'x'` on the next line stays two
    // statements.
    if (valid_symbols[COMMAND_LITERAL_START] && !valid_symbols[ERROR_SENTINEL]) {
        // A backslash at the end of the line continues it: `include \`.
        for (;;) {
            if (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                lexer->advance(lexer, true);
            } else if (lexer->lookahead == '\\') {
                lexer->advance(lexer, true);
                if (lexer->lookahead == '\r') {
                    lexer->advance(lexer, true);
                }
                if (lexer->lookahead != '\n') {
                    return false;
                }
                lexer->advance(lexer, true);
            } else {
                break;
            }
        }
        int32_t c = lexer->lookahead;
        if (c == '\'' || c == '"' || (c >= '0' && c <= '9') || c == '~') {
            lexer->mark_end(lexer);
            lexer->result_symbol = COMMAND_LITERAL_START;
            return true;
        }
        // A number with no digits before the point: `x .42` calls `x(.42)`.
        if (c == '.') {
            lexer->mark_end(lexer);
            advance(lexer);
            if (lexer->lookahead >= '0' && lexer->lookahead <= '9') {
                lexer->result_symbol = COMMAND_LITERAL_START;
                return true;
            }
            return false;
        }
        // `!x`, not `!=`, `!in` or `!instanceof`, which continue the receiver.
        if (c == '!') {
            lexer->mark_end(lexer);
            advance(lexer);
            if (lexer->lookahead == '=') {
                return false;
            }
            if (lexer->lookahead == 'i') {
                char word[12];
                unsigned len = read_word(lexer, word, sizeof(word));
                if ((len == 2 && memcmp(word, "in", 2) == 0)
                    || (len == 10 && memcmp(word, "instanceof", 10) == 0)) {
                    return false;
                }
            }
            lexer->result_symbol = COMMAND_LITERAL_START;
            return true;
        }
        // `new`, `null`, `true` and `false` start an argument too, since none
        // can be a declaration's name: `respond new Foo(), model: m`,
        // `assertNull null.x()`. So does a name followed by `.` or `?.` and
        // another name, since a declared name has no dot: `println it.name`,
        // `reply message.reverse()`. Any other name is left to the
        // named-key branch.
        if (is_ident_start(c)) {
            lexer->mark_end(lexer);
            char word[12];
            unsigned len = read_word(lexer, word, sizeof(word));
            if ((len == 3 && memcmp(word, "new", 3) == 0)
                || (len == 4 && memcmp(word, "null", 4) == 0)
                || (len == 4 && memcmp(word, "true", 4) == 0)
                || (len == 5 && memcmp(word, "false", 5) == 0)) {
                // `foo new: 1` names an argument `new`.
                if (lexer->lookahead == ':' && valid_symbols[COMMAND_NAMED_KEY]) {
                    lexer->mark_end(lexer);
                    advance(lexer);
                    if (lexer->lookahead != ':') {
                        lexer->result_symbol = COMMAND_NAMED_KEY;
                        return true;
                    }
                    return false;
                }
                lexer->result_symbol = COMMAND_LITERAL_START;
                return true;
            }
            // `println as` at the end of its line (see finish_command_named_key).
            if (valid_symbols[COMMAND_KEYWORD_ARGUMENT] && len == 2
                && (memcmp(word, "as", 2) == 0 || memcmp(word, "in", 2) == 0)
                && (lexer->lookahead == '\n' || lexer->lookahead == '\r')) {
                lexer->mark_end(lexer);
                lexer->result_symbol = COMMAND_KEYWORD_ARGUMENT;
                return true;
            }
            // `tag List<String>`: a generic type as the argument (Groovy 3
            // and 4), since a declared name takes no type arguments.
            if (lexer->lookahead == '<') {
                if (!type_arguments_end_argument(lexer)) {
                    return false;
                }
                lexer->result_symbol = COMMAND_LITERAL_START;
                return true;
            }
            // `if (b) p.accept f(t())` then `else`: a call as a braceless if
            // body's one argument, which elsewhere reads as two statements or
            // a method declaration; only an `else` after it decides.
            if (lexer->lookahead == '(' && is_name_argument(word, len)) {
                advance(lexer);
                if (skip_parens(lexer) && else_follows(lexer)) {
                    lexer->result_symbol = COMMAND_LITERAL_START;
                    return true;
                }
                return false;
            }
            // `println as[0]`: a variable named `as` (Groovy 3 and 4), since
            // no type starts with `[`.
            if (len == 2 && memcmp(word, "as", 2) == 0 && lexer->lookahead == '[') {
                lexer->result_symbol = COMMAND_LITERAL_START;
                return true;
            }
            if (lexer->lookahead == '.' || lexer->lookahead == '?') {
                bool safe = lexer->lookahead == '?';
                advance(lexer);
                if (!safe || lexer->lookahead == '.') {
                    if (safe) {
                        advance(lexer);
                    }
                    if (is_ident_start(lexer->lookahead)) {
                        lexer->result_symbol = COMMAND_LITERAL_START;
                        return true;
                    }
                }
                return false;
            }
            // A name that ends its line, where a key can't be. Groovy carries a
            // later line that starts with `.`, `?.` or `*.` on from it: `a.foo bar`
            // then `.baz` is `a.foo(bar.baz)`, so the name starts an argument.
            // Otherwise it is the name argument that ends a statement (`a.b key`),
            // given here as the zero-width `_command_name_start` before the name,
            // since the lexer has looked past the end of `_command_name_argument`.
            if (lexer->lookahead == '\n' || lexer->lookahead == '\r') {
                while (is_whitespace(lexer->lookahead)) {
                    advance(lexer);
                }
                // `x.url isSnapshot` then `? "a"` then `: "b"`, or then `?: 2`:
                // a line can't start with `?`, so a ternary or elvis there
                // continues the argument, as Groovy reads it.
                if (lexer->lookahead == '?') {
                    advance(lexer);
                    if (lexer->lookahead != '.') {
                        lexer->result_symbol = COMMAND_LITERAL_START;
                        return true;
                    }
                } else if (lexer->lookahead == '*') {
                    advance(lexer);
                }
                if (lexer->lookahead == '.') {
                    advance(lexer);
                    // also `.@field` and `.&method`
                    if (is_ident_start(lexer->lookahead) || lexer->lookahead == '@'
                        || lexer->lookahead == '&') {
                        lexer->result_symbol = COMMAND_LITERAL_START;
                        return true;
                    }
                }
                if (!valid_symbols[COMMAND_NAME_START] || !is_name_argument(word, len)) {
                    return false;
                }
                lexer->result_symbol = COMMAND_NAME_START;
                return true;
            }
            if (valid_symbols[COMMAND_NAMED_KEY] || valid_symbols[COMMAND_NAME_ARGUMENT]
                || valid_symbols[CLOSURE_COMMAND_ARGUMENT] || valid_symbols[COMMAND_OPERAND]) {
                return finish_command_named_key(lexer, valid_symbols, word, len);
            }
            return false;
        }
    }

    // The same for a closure after a bare identifier receiver, so the
    // closure may lead a longer argument expression.
    if (valid_symbols[COMMAND_CLOSURE_START] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
        }
        if (lexer->lookahead == '{') {
            lexer->mark_end(lexer);
            lexer->result_symbol = COMMAND_CLOSURE_START;
            return true;
        }
    }

    if ((valid_symbols[COMMAND_NAMED_KEY] || valid_symbols[COMMAND_NAME_ARGUMENT])
        && !valid_symbols[ERROR_SENTINEL]) {
        // Where no statement may end (after a comma between arguments), the
        // key may start on the next line: `sh returnStdout: true,` then
        // `script: 'x'`. Elsewhere a key on the next line is a label.
        // A backslash at the end of a line continues it: `f a: 1, \` then `b: 2`.
        for (;;) {
            if (lexer->lookahead == ' ' || lexer->lookahead == '\t'
                || (!valid_symbols[LINE_BREAK]
                    && (lexer->lookahead == '\n' || lexer->lookahead == '\r'))) {
                newline |= lexer->lookahead == '\n' || lexer->lookahead == '\r';
                lexer->advance(lexer, true);
            } else if (lexer->lookahead == '\\') {
                lexer->advance(lexer, true);
                if (lexer->lookahead == '\r') {
                    lexer->advance(lexer, true);
                }
                if (lexer->lookahead != '\n') {
                    return false;
                }
                lexer->advance(lexer, true);
            } else {
                break;
            }
        }
        // An identifier is either the key or, once consumed, not something
        // the code below could start (a comment or slashy string), so stop
        // here. Falling through after the scan advanced past `b` in
        // `foo a, b` and a newline made the comment branch emit `b` as a
        // line_comment, since mark_end had already been called.
        if (is_ident_start(lexer->lookahead)) {
            return scan_command_named_key(lexer, valid_symbols);
        }
    }

    // SPECIFICATION.md §5.5 — zero-width, before the first argument of a
    // command call that is a declaration's or assignment's value: a name,
    // string or number on the receiver's line. A name that continues the
    // expression (`as`, `in`, `instanceof`) or the statement (`else`) is not
    // an argument.
    if (valid_symbols[COMMAND_VALUE_START] && !valid_symbols[ERROR_SENTINEL]) {
        // A backslash at the end of the line continues it: `def r = sh \`
        // then `script: 'x'`.
        for (;;) {
            if (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                lexer->advance(lexer, true);
            } else if (lexer->lookahead == '\\') {
                lexer->advance(lexer, true);
                if (lexer->lookahead == '\r') {
                    lexer->advance(lexer, true);
                }
                if (lexer->lookahead != '\n') {
                    return false;
                }
                lexer->advance(lexer, true);
            } else {
                break;
            }
        }
        int32_t c = lexer->lookahead;
        if (c == '\'' || c == '"' || (c >= '0' && c <= '9')) {
            lexer->mark_end(lexer);
            lexer->result_symbol = COMMAND_VALUE_START;
            return true;
        }
        if (is_ident_start(c)) {
            lexer->mark_end(lexer);
            char word[12];
            unsigned len = 0;
            while (is_ident_part(lexer->lookahead)) {
                if (len < sizeof(word)) {
                    word[len] = (char)lexer->lookahead;
                }
                len++;
                advance(lexer);
            }
            if ((len == 2 && (memcmp(word, "as", 2) == 0 || memcmp(word, "in", 2) == 0))
                || (len == 4 && memcmp(word, "else", 4) == 0)
                || (len == 10 && memcmp(word, "instanceof", 10) == 0)) {
                return false;
            }
            lexer->result_symbol = COMMAND_VALUE_START;
            return true;
        }
    }

    // SPECIFICATION.md §5.10 — zero-width, before a statement that starts on
    // a new line with a name or `(`, where the line before may end one. The
    // names that continue a statement or expression across a line break are
    // left alone: `else`, `catch`, `finally`, `while` (of `do`), `case`,
    // `default`, `as`, `in`, `instanceof`, `throws`, `extends`, `implements`,
    // `permits`.
    if (valid_symbols[LINE_BREAK] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
        }
        if (lexer->lookahead == '\n' || lexer->lookahead == '\r') {
            newline = skip_whitespace(lexer);
            // A `{` on the line after `new X()` where a statement may end
            // starts a closure argument, not a class body.
            if (lexer->lookahead == '{' && valid_symbols[NEW_CLOSURE_START]) {
                lexer->mark_end(lexer);
                lexer->result_symbol = NEW_CLOSURE_START;
                return true;
            }
            if (lexer->lookahead == '(') {
                lexer->mark_end(lexer);
                lexer->result_symbol = LINE_BREAK;
                return true;
            }
            // `c` then `?[x]` then `:y`, the ternary of Groovy 2.4 and 3 over lines;
            // no statement starts with `?`
            if (lexer->lookahead == '?') {
                if (!valid_symbols[TERNARY_QUESTION] || valid_symbols[EXPRESSION_COLON]) {
                    return false;
                }
                advance(lexer);
                lexer->mark_end(lexer);
                if (lexer->lookahead == '[' && bracket_then_colon(lexer)) {
                    lexer->result_symbol = TERNARY_QUESTION;
                    return true;
                }
                return false;
            }
            if (is_ident_start(lexer->lookahead)) {
                lexer->mark_end(lexer);
                char word[12];
                unsigned len = 0;
                while (is_ident_part(lexer->lookahead)) {
                    if (len < sizeof(word)) {
                        word[len] = (char)lexer->lookahead;
                    }
                    len++;
                    advance(lexer);
                }
                static const char *const continuing[] = {
                    "else", "catch", "finally", "while", "case", "default",
                    "as", "in", "instanceof", "throws", "extends",
                    "implements", "permits",
                };
                // `in.property`: a name before a dot starts a statement, and
                // `in = 2` assigns to a variable named `in` (Groovy 3 and 4).
                // So does `as = 2`, and `in` or `as` alone on its line, or
                // `as[0]`, which no type starts.
                bool dotted = lexer->lookahead == '.';
                if (len == 2 && (memcmp(word, "in", 2) == 0 || memcmp(word, "as", 2) == 0)) {
                    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                        advance(lexer);
                    }
                    int32_t n = lexer->lookahead;
                    dotted |= n == '=' || n == '\n' || n == '\r' || n == ';' || n == '}'
                        || lexer->eof(lexer) || (word[0] == 'a' && n == '[');
                }
                for (unsigned i = 0; i < sizeof(continuing) / sizeof(continuing[0]) && !dotted; i++) {
                    if (strlen(continuing[i]) == len && memcmp(word, continuing[i], len) == 0) {
                        return false;
                    }
                }
                lexer->result_symbol = LINE_BREAK;
                return true;
            }
        }
    }

    // SPECIFICATION.md §4 — `var` and `trait` are keywords only before a name
    // on the same line (not `in`, `as` or `instanceof`, which make them a
    // value), `pipeline` only before a `{` on its line, and `sealed` only
    // before `class`, `interface` or `trait`. Otherwise the grammar lexes the
    // word as an identifier: `var.set(1)`, `{ var -> }`, `def var = 1`,
    // `trait.parentFile`, `Pipeline pipeline = p`, `def sealed = 1`.
    if ((valid_symbols[VAR_KEYWORD] || valid_symbols[PIPELINE_KEYWORD]
         || valid_symbols[TRAIT_KEYWORD] || valid_symbols[YIELD_KEYWORD]
         || valid_symbols[SEALED_KEYWORD])
        && !valid_symbols[ERROR_SENTINEL]) {
        newline |= skip_whitespace(lexer);
        int32_t c = lexer->lookahead;
        if (c == 'v' || c == 'p' || c == 't' || c == 'y' || c == 's') {
            lexer->mark_end(lexer);
            char word[12];
            unsigned len = 0;
            while (is_ident_part(lexer->lookahead)) {
                if (len < sizeof(word)) {
                    word[len] = (char)lexer->lookahead;
                }
                len++;
                advance(lexer);
            }
            enum TokenType sym;
            if (len == 3 && memcmp(word, "var", 3) == 0 && valid_symbols[VAR_KEYWORD]) {
                sym = VAR_KEYWORD;
            } else if (len == 8 && memcmp(word, "pipeline", 8) == 0
                && valid_symbols[PIPELINE_KEYWORD]) {
                sym = PIPELINE_KEYWORD;
            } else if (len == 5 && memcmp(word, "trait", 5) == 0
                && valid_symbols[TRAIT_KEYWORD]) {
                sym = TRAIT_KEYWORD;
            } else if (len == 5 && memcmp(word, "yield", 5) == 0
                && valid_symbols[YIELD_KEYWORD]) {
                sym = YIELD_KEYWORD;
            } else if (len == 6 && memcmp(word, "sealed", 6) == 0
                && valid_symbols[SEALED_KEYWORD]) {
                sym = SEALED_KEYWORD;
            } else {
                sym = ERROR_SENTINEL;
            }
            if (sym == ERROR_SENTINEL || lexer->lookahead == '.') {
                return valid_symbols[CLOSURE_DECLARATION_START]
                    && finish_closure_declaration(lexer, c,
                        (len == 4 && memcmp(word, "this", 4) == 0)
                        || (len == 5 && memcmp(word, "super", 5) == 0));
            }
            lexer->mark_end(lexer);
            bool spaced = lexer->lookahead == ' ' || lexer->lookahead == '\t';
            while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                advance(lexer);
            }
            c = lexer->lookahead;
            if (sym == YIELD_KEYWORD) {
                // `yield` starts a yield statement only before the start of a
                // value on its line: a name, a literal, a closure, a unary
                // operator, or `(` or `[` after a space. Otherwise it is a
                // name: `yield.call(a)`, `yield(1)`, `{ yield -> }`,
                // `yield = 1`, `yield[0]`.
                bool value = is_ident_start(c) || (c >= '0' && c <= '9')
                    || c == '\'' || c == '"' || c == '{'
                    || ((c == '(' || c == '[') && spaced);
                if (c == '-' || c == '+' || c == '!' || c == '~') {
                    advance(lexer);
                    // not `->`, `-=`, `+=` or `!=`
                    value = lexer->lookahead != '=' && !(c == '-' && lexer->lookahead == '>');
                }
                if (!value) {
                    return false;
                }
                if (is_ident_start(c)) {
                    len = 0;
                    while (is_ident_part(lexer->lookahead)) {
                        if (len < sizeof(word)) {
                            word[len] = (char)lexer->lookahead;
                        }
                        len++;
                        advance(lexer);
                    }
                    if ((len == 2 && (memcmp(word, "in", 2) == 0 || memcmp(word, "as", 2) == 0))
                        || (len == 10 && memcmp(word, "instanceof", 10) == 0)) {
                        return false;
                    }
                }
            } else if (sym == SEALED_KEYWORD) {
                // Before a type's keyword, also on a later line, or before a
                // modifier or an annotation, which Groovy 4 takes after it
                // (`sealed abstract class W`, `sealed @Deprecated class W`).
                // Before `final` or `non-sealed` it stays an error.
                while (is_whitespace(lexer->lookahead)) {
                    advance(lexer);
                }
                if (lexer->lookahead != '@') {
                    static const char *const after[] = {
                        "class", "interface", "trait", "record", "enum", "non",
                        "final", "abstract", "static", "public", "private",
                        "protected", "strictfp",
                    };
                    len = read_word(lexer, word, sizeof(word));
                    bool keyword = false;
                    for (unsigned i = 0; i < sizeof(after) / sizeof(after[0]) && !keyword; i++) {
                        keyword = strlen(after[i]) == len && memcmp(word, after[i], len) == 0;
                    }
                    if (!keyword) {
                        return false;
                    }
                }
            } else if (sym == PIPELINE_KEYWORD) {
                if (lexer->lookahead != '{') {
                    return false;
                }
                // Only when the `}` ends the statement: `pipeline { a } | b`,
                // `pipeline { }.with { }` call a method named `pipeline`.
                // An unbalanced body keeps the block reading.
                unsigned budget = SKIP_BUDGET;
                if (skip_braces(lexer, &budget)) {
                    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                        advance(lexer);
                    }
                    c = lexer->lookahead;
                    bool ends = lexer->eof(lexer) || c == '\n' || c == '\r' || c == ';'
                        || c == '}';
                    if (c == '/') {
                        advance(lexer);
                        ends = lexer->lookahead == '/' || lexer->lookahead == '*';
                    }
                    if (!ends) {
                        return false;
                    }
                }
            } else {
                if (!is_ident_start(lexer->lookahead)) {
                    return false;
                }
                len = 0;
                while (is_ident_part(lexer->lookahead)) {
                    if (len < sizeof(word)) {
                        word[len] = (char)lexer->lookahead;
                    }
                    len++;
                    advance(lexer);
                }
                if ((len == 2 && (memcmp(word, "in", 2) == 0 || memcmp(word, "as", 2) == 0))
                    || (len == 10 && memcmp(word, "instanceof", 10) == 0)) {
                    return false;
                }
                // `var message: 'Hi'` at the start of a statement is a call
                // with a named argument; `for (var x : xs)` declares `x`.
                // A record may start where a statement does, not in a `for`.
                if (sym == VAR_KEYWORD && valid_symbols[RECORD_KEYWORD]) {
                    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                        advance(lexer);
                    }
                    if (lexer->lookahead == ':') {
                        advance(lexer);
                        if (lexer->lookahead != ':') {
                            return false;
                        }
                    }
                }
            }
            lexer->result_symbol = sym;
            return true;
        }
    }

    // SPECIFICATION.md §4 — `record` starts a record declaration only when
    // a name and then `(` or `<` follow on the same line. Otherwise it is
    // left to the grammar, which lexes it as an identifier.
    if (valid_symbols[RECORD_KEYWORD] && !valid_symbols[ERROR_SENTINEL]) {
        newline |= skip_whitespace(lexer);
        if (lexer->lookahead == 'r') {
            lexer->mark_end(lexer);
            bool record = true;
            for (const char *p = "record"; *p && record; p++) {
                record = lexer->lookahead == *p;
                if (record) {
                    advance(lexer);
                }
            }
            if (!record || is_ident_part(lexer->lookahead) || lexer->lookahead == '.') {
                while (is_ident_part(lexer->lookahead)) {
                    advance(lexer);
                }
                return valid_symbols[CLOSURE_DECLARATION_START]
                    && finish_closure_declaration(lexer, 'r', false);
            }
            lexer->mark_end(lexer);
            if (lexer->lookahead != ' ' && lexer->lookahead != '\t') {
                return false;
            }
            while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                advance(lexer);
            }
            if (!is_ident_start(lexer->lookahead)) {
                return false;
            }
            while (is_ident_part(lexer->lookahead)) {
                advance(lexer);
            }
            while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                advance(lexer);
            }
            if (lexer->lookahead == '(' || lexer->lookahead == '<') {
                lexer->result_symbol = RECORD_KEYWORD;
                return true;
            }
            return false;
        }
    }

    // The declaration body (see finish_closure_declaration). A name starting
    // with `v`, `p`, `t`, `y` or `r` was handed on by the keyword scans above.
    if (valid_symbols[CLOSURE_DECLARATION_START] && !valid_symbols[ERROR_SENTINEL]) {
        newline |= skip_whitespace(lexer);
        int32_t c = lexer->lookahead;
        if (is_ident_start(c)) {
            lexer->mark_end(lexer);
            char word[6];
            unsigned len = read_word(lexer, word, sizeof(word));
            bool self = (len == 4 && memcmp(word, "this", 4) == 0)
                || (len == 5 && memcmp(word, "super", 5) == 0);
            return finish_closure_declaration(lexer, c, self);
        }
    }

    // SPECIFICATION.md §3.2 — `!in` right before `[`, `(` or `{`:
    // `1 !in[2, 3]`. The grammar's own `!in` token needs a character after
    // it and would take the bracket; this one ends before it.
    if (valid_symbols[NOT_IN_OPEN] && !valid_symbols[ERROR_SENTINEL]) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, true);
        }
        if (lexer->lookahead == '!') {
            for (const char *p = "!in"; *p; p++) {
                if (lexer->lookahead != *p) {
                    return false;
                }
                advance(lexer);
            }
            int32_t c = lexer->lookahead;
            if (c != '[' && c != '(' && c != '{') {
                return false;
            }
            lexer->result_symbol = NOT_IN_OPEN;
            return true;
        }
    }

    // SPECIFICATION.md §6.6 — line / block / groovydoc comments
    // and §6.2 slashy strings. Both start with `/`; combining their
    // dispatch keeps the in-grammar lexer from partially consuming
    // the leading `/`. Leading whitespace is skipped via
    // advance(true) so the scanner gets a chance to fire after the
    // in-grammar `\s` SKIP rule — without this, tree-sitter's lexer
    // eats `\n`, does NOT re-invoke the external scanner, then
    // fails on the following `/` and consumes it through error
    // recovery.
    bool wants_comment = valid_symbols[LINE_COMMENT]
        || valid_symbols[BLOCK_COMMENT]
        || valid_symbols[GROOVYDOC_COMMENT];
    bool wants_slashy = valid_symbols[SLASHY_STRING_START];
    if (!wants_comment && !wants_slashy) {
        return false;
    }

    // A comment may follow a `\` line continuation: `a || \` then `// c`.
    bool stray = false;
    newline |= skip_whitespace_and_continuations(lexer, &stray);
    // SPECIFICATION.md §6.6 — a Hubitat `#include ns.Library` line, which
    // the hub expands before compiling, is read as a line comment when it
    // starts at column 0, so the rest of the file still parses.
    if (!stray && lexer->lookahead == '#' && valid_symbols[LINE_COMMENT]
        && lexer->get_column(lexer) == 0) {
        for (const char *p = "#include "; *p; p++) {
            if (lexer->lookahead != *p) {
                return false;
            }
            advance(lexer);
        }
        while (!lexer->eof(lexer) && lexer->lookahead != '\n' && lexer->lookahead != '\r') {
            advance(lexer);
        }
        lexer->result_symbol = LINE_COMMENT;
        return true;
    }
    // SPECIFICATION.md §5.10 — where no statement may end, inside `( … )` or
    // `[ … ]`, a line break is whitespace, so a `[` or `(` on the next line
    // still indexes or calls, as in Groovy 2.4, 3 and 4: `f(a.x` then `[0])`,
    // `f(1, Math.max` then `(2, 3))`, `[ []` then `[1] ]` (`[][[1]]`).
    if (newline && !stray && !valid_symbols[LINE_BREAK] && !valid_symbols[ERROR_SENTINEL]) {
        if (lexer->lookahead == '[' && valid_symbols[SUBSCRIPT_OPEN]) {
            advance(lexer);
            lexer->mark_end(lexer);
            // Not an empty `[]`, which would make an array type of a dotted name on
            // the line before: a Spock row `a | X.Y` then `[] | b`.
            while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                advance(lexer);
            }
            if (lexer->lookahead == ']') {
                return false;
            }
            lexer->result_symbol = SUBSCRIPT_OPEN;
            return true;
        }
        if (lexer->lookahead == '(' && valid_symbols[CALL_OPEN]) {
            advance(lexer);
            lexer->result_symbol = CALL_OPEN;
            return true;
        }
    }
    if (stray || lexer->lookahead != '/') {
        return false;
    }

    advance(lexer);
    // Dispatch on the char after `/`:
    //   `/`  → line comment
    //   `*`  → block / groovydoc comment
    //   `=`  → augmented assign, in-grammar handles it
    //   else → slashy string (when valid_symbols allow)
    int32_t next = lexer->lookahead;
    if (next == '/') {
        if (!valid_symbols[LINE_COMMENT]) {
            return false;
        }
        advance(lexer);
        // EOF-bounded loop: lexer->eof() guarantees termination on
        // unterminated input. Do not tighten without preserving the
        // EOF guard.
        // A lone `\r` ends a line too (classic Mac line endings).
        while (!lexer->eof(lexer) && lexer->lookahead != '\n' && lexer->lookahead != '\r') {
            advance(lexer);
        }
        lexer->result_symbol = LINE_COMMENT;
        return true;
    }
    if (next == '*') {
        if (!wants_comment) {
            return false;
        }
        advance(lexer);
        bool is_groovydoc = false;
        if (lexer->lookahead == '*') {
            advance(lexer);
            is_groovydoc = true;
            if (lexer->lookahead == '/') {
                advance(lexer);
                if (!valid_symbols[GROOVYDOC_COMMENT]) {
                    return false;
                }
                lexer->result_symbol = GROOVYDOC_COMMENT;
                return true;
            }
        }
        // EOF-bounded loop: a `/* …` without a closer terminates
        // here via lexer->eof(), not by going past end-of-input.
        while (!lexer->eof(lexer)) {
            if (lexer->lookahead == '*') {
                advance(lexer);
                if (lexer->lookahead == '/') {
                    advance(lexer);
                    if (is_groovydoc) {
                        if (!valid_symbols[GROOVYDOC_COMMENT]) {
                            return false;
                        }
                        lexer->result_symbol = GROOVYDOC_COMMENT;
                    } else {
                        if (!valid_symbols[BLOCK_COMMENT]) {
                            return false;
                        }
                        lexer->result_symbol = BLOCK_COMMENT;
                    }
                    return true;
                }
            } else {
                advance(lexer);
            }
        }
        return false;
    }
    if (!wants_slashy) {
        return false;
    }
    // Groovy reads a `/` right after an operand as division, never as the
    // start of a slashy string: `a/2 + b/2`, `"${t/1000f},${n/1000f}"`. An
    // operand has just ended where a subscript `[` could follow, unless a
    // line ended in between where a statement may end: `x` then `/re/`.
    if (valid_symbols[SUBSCRIPT_OPEN] && !valid_symbols[ERROR_SENTINEL]
        && (!newline || !valid_symbols[LINE_BREAK])) {
        return false;
    }
    // Where an expression must start (after `=~`, `~`, `(`, `,` or an
    // operator) and no statement or command argument could start instead,
    // a `/` can't be division: the body may then start with whitespace
    // (`=~ / X /`, `split(/ /)`) and run over several lines (`def s = /`).
    bool lenient = !valid_symbols[LINE_BREAK] && !valid_symbols[COMMAND_LITERAL_START]
        && !valid_symbols[COMMAND_NAMED_KEY] && !valid_symbols[COMMAND_VALUE_START]
        && !valid_symbols[ERROR_SENTINEL];
    if (next == '=' && !lenient) {
        // `/=` is the augmented-assign operator. The in-grammar
        // lexer owns it. Returning false here causes tree-sitter
        // to rewind the cursor to before the leading `/`, so the
        // in-grammar lexer can match `/=` cleanly. Where an
        // expression must start it opens a slashy: `split(/=/, 2)`.
        return false;
    }
    // Slashy string opening `/` — we've consumed it; the body and
    // closing `/` are matched by the grammar's `slashy_string` rule
    // via regex tokens, so `$identifier` and `${expr}` segments can
    // be exposed as structured children. SPECIFICATION.md §5.4 /
    // §6.2 require a context-sensitive emit. Three pragmatic guards
    // keep `a / b` and similar division contexts from being
    // mis-tokenised as the opening of a slashy:
    //
    //   (1) Reject an empty body (the next char after the opening
    //       `/` is a closing `/`) — `//` is already a line comment
    //       and `/=` is already trapped above, so an empty slashy
    //       is excluded.
    //   (2) Reject a body that starts with whitespace — `a / b`
    //       has a space immediately after the operator `/`, but
    //       every legal slashy regex starts with a non-whitespace
    //       char.
    //   (3) Look ahead through the body to confirm there is a
    //       closing `/` on the same line. Without this, `a / b\nc`
    //       at end of file would emit a slashy opener and leave
    //       the body unterminated.
    //       At the start of a line where a statement may start, Groovy
    //       reads a `/` and a closing `/` on that line as a slashy
    //       string whatever follows the `/`: `a | b` then `/ x / | c`
    //       (a Spock where: row). Without the closing `/` it is division:
    //       `x = 4` then `/ 2`.
    bool line_start = newline && valid_symbols[LINE_BREAK] && !valid_symbols[ERROR_SENTINEL];
    if (lexer->lookahead == '/' || (is_whitespace(lexer->lookahead) && !lenient && !line_start)) {
        return false;
    }
    // Look ahead (without consuming) to confirm a closing `/` exists
    // before a newline or EOF. `lexer->mark_end` records the position
    // where the slashy opener token ends; advancing past it during
    // look-ahead does not extend the token range as long as we do
    // not call `mark_end` again.
    lexer->mark_end(lexer);
    bool found_close = false;
    bool crossed = false;
    while (!lexer->eof(lexer)) {
        // `\/` is an escaped slash; any other backslash is one character.
        // A backslash at the end of a line continues it: `/abc \` then ` x/`.
        if (lexer->lookahead == '\\') {
            advance(lexer);
            if (lexer->lookahead == '\r') {
                advance(lexer);
            }
            if (lexer->lookahead == '/' || lexer->lookahead == '\n') {
                advance(lexer);
            }
            continue;
        }
        if (lexer->lookahead == '/') {
            found_close = true;
            break;
        }
        if ((lexer->lookahead == '\n' || lexer->lookahead == '\r') && !lenient) {
            // At the start of a statement it may run over lines too, as Groovy
            // 3 and 4 read it: `/ *` then ` * text` then `/`. Only when its `/`
            // then ends the line, so `x = 4` then `/ 2` stays a division
            // where a later line holds a `/`.
            if (!line_start) {
                break;
            }
            crossed = true;
        }
        advance(lexer);
    }
    if (found_close && crossed) {
        advance(lexer);
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            advance(lexer);
        }
        found_close = lexer->eof(lexer) || lexer->lookahead == '\n' || lexer->lookahead == '\r'
            || lexer->lookahead == ';';
    }
    if (!found_close) {
        return false;
    }
    lexer->result_symbol = SLASHY_STRING_START;
    return true;
}
