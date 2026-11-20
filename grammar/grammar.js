/**
 * @file Apache Groovy grammar for tree-sitter
 * @author Elijah Zupancic <elijah@zupancic.name>
 * @license MIT
 *
 * The authoritative design document for this grammar is
 * SPECIFICATION.md at the repository root. Every rule below maps to
 * a section there. Read the spec before making non-trivial changes.
 *
 * The external scanner (src/scanner.c) handles four context-sensitive
 * lexing situations that pure grammar rules cannot disambiguate:
 *   - Slashy-string-start vs. division
 *   - Automatic-semicolon (newline-as-terminator)
 *   - Labelled-statement colon
 *   - Block-comment / Groovydoc single-token emit
 * See SPECIFICATION.md §6.
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

/* eslint-disable no-multi-spaces --
 * The aligned trailing comments make the Apache-mirror precedence
 * ladder readable at a glance.
 */
const PREC = {
  // Higher number = tighter binding in tree-sitter.
  // Mirrors apache/groovy:GroovyParser.g4 `expression` rule.
  // See SPECIFICATION.md §3.1.
  ASSIGN: 1,                  // = += -= *= /= %= **= <<= >>= >>>= &= ^= |= ?=
  CONDITIONAL: 2,             // ?: (Elvis) and ? : (ternary) — same level
  IMPLICATION: 3,             // ==>
  LOGICAL_OR: 4,              // ||
  LOGICAL_AND: 5,             // &&
  BITWISE_OR: 6,              // |
  BITWISE_XOR: 7,             // ^
  BITWISE_AND: 8,             // &
  EQUALITY: 9,                // == != <=> === !== =~ ==~
  RELATIONAL: 10,             // < <= > >= in !in instanceof !instanceof as
  SHIFT_OR_RANGE: 11,         // << >> >>> .. ..< <.. <..<
  ADDITIVE: 12,               // + -
  MULTIPLICATIVE: 13,         // * / %
  UNARY: 14,                  // prefix ++ -- + -          (Apache level 3 UNARY_ADD)
  POWER: 15,                  // **                         (Apache level 2)
  UNARY_NOT: 16,              // ! ~                        (Apache level 1, tighter than POWER)
  POSTFIX: 17,                // postfix ++ --, []
  ACCESS: 18,                 // . ?. ??. *. .& .@ ::
  PRIMARY: 19,                // literals, identifiers, ()
  // Type-context only — never competes with the expression ladder
  // above. Shares CONDITIONAL's numeric value by accident; the two
  // do not collide because `_type` and `_expression` reach the
  // resolver through disjoint parse stacks.
  ARRAY_TYPE: 2,
};
/* eslint-enable no-multi-spaces */

/**
 * The binary operators at their precedence levels, each a `binary_expression`
 * whose right operand is `right`.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral} right The right operand.
 * @param {RuleOrLiteral} [left] The left operand.
 * @returns {ChoiceRule} One alternative per operator.
 */
function binaryOperators($, right, left = $._expression) {
  /* eslint-disable no-multi-spaces --
   * Aligned columns make the precedence ladder readable.
   */
  const ops = [
    // ADDITIVE — §3.1 level 5
    ['+',   PREC.ADDITIVE],
    ['-',   PREC.ADDITIVE],
    // MULTIPLICATIVE — §3.1 level 4
    ['*',   PREC.MULTIPLICATIVE],
    ['/',   PREC.MULTIPLICATIVE],
    ['%',   PREC.MULTIPLICATIVE],
    // SHIFT — §3.1 level 6. Ranges (`..`, `..<`, etc.) share
    // this precedence level but use distinct node kinds, so
    // they live in their own rule (lands later).
    ['<<',  PREC.SHIFT_OR_RANGE],
    ['>>',  PREC.SHIFT_OR_RANGE],
    ['>>>', PREC.SHIFT_OR_RANGE],
    // RELATIONAL — §3.1 level 7. `in`, `!in`, `instanceof`,
    // `!instanceof`, and `as` share this level but emit
    // distinct node kinds (`membership_expression`,
    // `instanceof_expression`, `cast_expression`), so they
    // land in their own rules later.
    ['<',   PREC.RELATIONAL],
    // §5.14 — the scanner's `<` when what follows can't be type
    // arguments, where a type's `<` could also follow the name:
    // `i<n ? a : b`, `(i<a.size())`.
    [alias($._less_than, '<'), PREC.RELATIONAL],
    ['<=',  PREC.RELATIONAL],
    ['>',   PREC.RELATIONAL],
    ['>=',  PREC.RELATIONAL],
    // EQUALITY — §3.1 level 8. `===`, `!==`, `<=>`, `=~`,
    // `==~` share this level but emit distinct node kinds.
    ['==',  PREC.EQUALITY],
    ['!=',  PREC.EQUALITY],
    // BITWISE — §3.1 levels 9, 10, 11 (each tighter than the
    // next: & < ^ < | by precedence).
    ['&',   PREC.BITWISE_AND],
    ['^',   PREC.BITWISE_XOR],
    ['|',   PREC.BITWISE_OR],
    // LOGICAL — §3.1 levels 12, 13 (&& tighter than ||).
    ['&&',  PREC.LOGICAL_AND],
    ['||',  PREC.LOGICAL_OR],
    // Groovy 6 null-coalescing, at the Elvis operator's level.
    ['??',  PREC.CONDITIONAL],
  ];
  /* eslint-enable no-multi-spaces */
  return choice(...ops.map(([op, p]) => prec.left(p, seq(
    field('left', left),
    field('operator', op),
    field('right', right),
  ))));
}

/**
 * GString body segment choice shared between `"..."` and `"""..."""`.
 * The text rule differs (single-quoted bans newlines, triple-quoted
 * allows them) but every other alternative is identical.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral} textRule The flavour-specific literal-text rule.
 * @returns {ChoiceRule} The shared body-segment choice.
 */
function gstringPart($, textRule) {
  return choice(
    alias(textRule, $.string_fragment),
    $.gstring_dollar_interpolation,
    $.gstring_brace_interpolation,
    alias($._gstring_literal_dollar, $.string_fragment),
    $._string_body,
  );
}

/**
 * A method's name, parameters, throws clause and body. A method without a
 * body (abstract, or in an interface) carries a negative dynamic
 * precedence: in a block, `(Type) x` followed by `setup(test)` on the next
 * line can also read as `(Type)` and a bodiless method `x setup(test)`.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The method tail.
 */
function methodNameToBody($) {
  return choice(
    seq(
      field('name', $._method_name),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
      field('body', $.block),
    ),
    prec.dynamic(-1, seq(
      field('name', $._method_name),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
    )),
  );
}

/**
 * The annotations and modifiers before a field or method. Annotations may
 * also follow modifiers: `final @Delegate Map target`,
 * `static @Rule TestName name`, `private static @Nullable String f()`.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {RuleOrLiteral[]} The prefix, to spread into a `seq`.
 */
function memberModifiers($) {
  return [
    repeat($.annotation),
    optional(seq(repeat1($._modifier), repeat1($.annotation))),
    repeat($._modifier),
  ];
}

/**
 * The modifiers after `def`, which annotations may lead or follow:
 * `def static x`, `def @Shared String x`, `def static @Lazy x`.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {RuleOrLiteral[]} The modifiers, to spread into a `seq`.
 */
// A closure after its `{`.
function closureBody($) {
  return [
    optional(seq(optional($._line_break), $.closure_parameters)),
    choice(
      repeat(choice($._statement, ';', $._line_break)),
      alias($._closure_command, $.command_chain),
      alias($._closure_declaration, $.local_variable_declaration),
    ),
    '}',
  ];
}

function defModifiers($) {
  return [
    repeat($._modifier),
    optional(seq(repeat1($.annotation), repeat($._modifier))),
  ];
}

/**
 * Annotations with modifiers before or after them, at least one annotation:
 * `@Field static`, `static @Field`.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The prefix.
 */
function annotatedModifiers($) {
  return choice(
    seq(repeat1($.annotation), repeat($._modifier)),
    seq(repeat1($._modifier), repeat1($.annotation), repeat($._modifier)),
  );
}

module.exports = grammar({
  name: 'groovy',

  // Placeholder for the real implementation. SPECIFICATION.md §3-§7
  // is the contract this rule set will fulfil.
  word: $ => $.identifier,

  // §5.2 — Groovy's statement keywords are never identifiers, except as a
  // property name after a dot (`x.class`) and as a map or named-argument
  // key (`[default: 1]`). Without this, a statement keyword after a newline
  // could lex as an identifier on one parse path: `List ids = (List) a.b`
  // followed by `if (ids) { … }` also read as `(List)` and a method named
  // `if` returning `a.b`.
  reserved: {
    global: _ => [
      'if', 'for', 'while', 'do', 'switch', 'case', 'return',
      'break', 'continue', 'throw', 'try', 'catch', 'finally', 'assert',
      'synchronized', 'new', 'class', 'interface', 'enum', 'import',
      'package', 'else',
    ],
    properties: _ => [],
  },

  extras: $ => [
    /\s/,
    // A backslash at the end of a line continues the statement on the next.
    /\\\r?\n/,
    // A byte order mark (some files start with two) is no token.
    /\uFEFF/,
    $.line_comment,
    $.block_comment,
    $.groovydoc_comment,
  ],

  // Externals listed here must have a matching branch in
  // `src/scanner.c` AND be referenced by at least one rule below.
  // Tokens the spec §6 proposes as external scanner branches but
  // that this grammar implements in-grammar (GString interpolation
  // via regex tokens, slashy body via in-grammar `_slashy_text`,
  // label colon via grammar precedence, and statement boundaries
  // via `repeat(choice($._statement, ';'))` — newline is `extras`
  // whitespace and `;` is an anonymous list element, see §5.10)
  // are intentionally NOT listed: listing an unwired external token
  // would let dormant scanner branches mis-fire in error-recovery
  // states.
  externals: $ => [
    $._slashy_string_start,
    $.line_comment,
    $.block_comment,
    $.groovydoc_comment,
    // §5.5 — `name:` key of a named argument in a parenthesis-free call.
    // Scanner-side so the newline before a Spock label (`result` then
    // `where:`) is not swallowed, and so `Type name =` stays a declaration.
    $._command_named_key,
    // §5.10 — `[` on the same line as the expression before it (spaces and
    // tabs allowed). A `[` after a newline isn't one, so it starts a list or
    // map literal: a statement, not a subscript of the line above.
    $._subscript_open,
    // §5.5 — zero-width: the first argument of a command call on a dotted
    // receiver (`response.setHeader 'k', v`) starts on the receiver's line.
    // A string or number on the next line is a new statement.
    $._command_literal_start,
    // §5.5 — zero-width: a `{` on a bare command receiver's line.
    $._command_closure_start,
    // §5.5 — `(` of an argument list, on the line of the expression
    // before it. A `(` after a newline starts a new statement. Parameter
    // lists accept it too, since `String f(` is also a call `f(` after a
    // statement `String`.
    $._call_open,
    // §5.15 — zero-width: a cast's value starts on the line of its `)`.
    $._cast_value_start,
    // §5.7 — never emitted. Valid only inside a string body, where it tells
    // the scanner not to read `//` or `/*` as a comment: `"// x"`.
    $._string_body,
    // §4 — `record` is a keyword only before a record's name and its `(` or
    // `<`; anywhere else it's an identifier: `record { }`, `return record`.
    $._record_keyword,
    // §5.5 — zero-width: the first argument of a command call that is the
    // value of a declaration or assignment starts on the receiver's line
    // (`def r = sh returnStdout: true, script: 'x'`, `x = foo bar`).
    $._command_value_start,
    // §5.10 — zero-width: a statement that starts on a new line with a name
    // or `(`. Where the line before could end a statement, this ends it, so
    // `x` and then `bar = 1` on the next line are two statements, never a
    // declaration `x bar = 1` typed by the line above.
    $._line_break,
    // §5.5 — the one argument of a command call on a dotted receiver when
    // the statement ends after it: `a.b key` (lexed like `_command_named_key`).
    $._command_name_argument,
    // §4 — `var`, `pipeline` and `trait` are keywords only where the shape
    // that follows makes them one: `var` and `trait` before a name on the
    // same line, `pipeline` before a `{`. Anywhere else each is an
    // identifier: `var.set(1)`, `{ var -> }`, `Pipeline pipeline = p`,
    // `trait.parentFile`.
    $._var_keyword,
    $._pipeline_keyword,
    $._trait_keyword,
    // §5.14 — the `<` of type arguments right after a name, when the scanner
    // finds a balanced, type-shaped `<…>` that a `)`, `,`, `]`, `.` or `::`
    // follows: `Mock(Supplier<Long>)`, `ArrayList<Job>.class`. Anywhere else
    // `<` is left to the grammar, so `i<n` stays a comparison.
    $._type_arguments_open,
    // §5.14 — a comparison's `<` right after a name, when no balanced,
    // type-shaped `<…>` follows: `i<n ? a : b`. Without it the in-grammar
    // `<` of type arguments wins where a generic type may start.
    $._less_than,
    // §4 — `yield` starts a yield statement only before a value on its line;
    // anywhere else it is a name: `yield.call(a)`, `yield(1)`, `{ yield -> }`.
    $._yield_keyword,
    // §3.2 — the `?` of a ternary whose first branch is a list or map right
    // after it, `c?[x: 1]:[x: 2]` (Groovy 2.4 and 3), when the balanced `[…]`
    // is followed by `:`. Otherwise `?[` is a safe index.
    $._ternary_question,
    // Never emitted: valid where a `:` may follow an expression (a ternary's
    // first branch, `case`, `assert`, a map key), which tells the scanner a
    // `?[…]:` there is a safe index before that `:`, not a ternary.
    $._expression_colon,
    // §5.5 — zero-width: `_command_name_argument` where the scanner had to look
    // past the line break after the name to tell it from the start of an
    // argument that continues on the next line (`a.foo bar` then `.baz`).
    $._command_name_start,
    // §4 — the `->` after a switch case's labels. A name right before it is
    // the label's last operand, never a lambda's parameter: `case FOO -> 1`,
    // `case a < b -> 3`.
    $._case_arrow,
    // The one name argument of a command call that is a closure's whole body,
    // when `}` follows it on its line: `{ x -> println x }` (lexed like
    // `_command_name_argument`).
    $._closure_command_argument,
    // The `(` of a lambda's parameters that is a command call's argument,
    // after a space: `configurations.all (Configuration it) -> { }`. The
    // scanner finds the balanced `(…)` and then `->`; otherwise it is a call's.
    $._command_lambda_open,
    // Zero-width: a closure's or block's whole body is a declaration with no
    // value, `{ Foo bar }`, not a command call (see _closure_declaration).
    $._closure_declaration_start,
    // §3.2 — `!in` right before `[`, `(` or `{`, ending before it.
    $._not_in_open,
    // §3.2 — zero-width: a `{` on the line after `new X()` where a statement
    // may end, which starts a closure argument (see method_invocation).
    $._new_closure_start,
    // §4 — `sealed` is a modifier only before `class`, `interface` or `trait`;
    // anywhere else it is a name: `def sealed = 1`, `sealed.message`.
    $._sealed_keyword,
    // §5.5 — `as` or `in` as a bare receiver's one argument, when the
    // statement ends after it: `println as` (lexed like `_command_name_argument`).
    $._command_keyword_argument,
    // §4 — zero-width: an array dimension's annotations, `String @A []`, only when
    // they and then `[` follow on the line (see `_dimension_annotations`).
    $._dimension_annotation_start,
    // §5.5 — the name an operator follows in a braceless if body's one command
    // argument, `if (c) o.f a - b` then `else` (see `_bare_operator_command`).
    $._command_operand,
    // §5.5 — zero-width: in a parenthesized command, the first argument is a
    // name an index or a binary operator follows on its line, which starts an
    // expression: `if (a.b c[0] - n)`, `while (a.b c - 1)`.
    $._paren_command_start,
    // Never referenced by a rule: valid only during error recovery, when
    // the scanner must not emit context-sensitive tokens.
    $._error_sentinel,
  ],

  conflicts: $ => [
    // `final def x` / `final x = 1` in a block vs. a modified member at top level.
    [$._modifier, $.local_variable_declaration],
    // `for (final int i; …)`: the for loop's bare declaration or a `final`
    // typed declaration, which read the same.
    [$._for_bare_declaration, $.local_variable_declaration],
    // `for (@A final x …`: a for-in loop's modifiers or an annotated declaration.
    [$._modifier, $.local_variable_declaration, $.for_in_statement],
    // `for (int i, j; …)` and `for (int i, int j in …)` split at the comma.
    [$._bare_declarator, $.for_in_statement],
    // `if (a) foo b bar c` or `if (a) render x as JSON` then `else`: a
    // chain of names or a cast argument, or statements.
    [$.command_chain, $._bare_name_command, $._bare_cast_command, $._expression, $._type],
    [$.command_chain, $._bare_name_command, $._bare_cast_command, $._type],
    [$._bare_cast_argument, $._expression],
    [$._modifier, $.multi_assignment_declaration],
    [$._modifier, $.for_in_statement],
    [$._modifier, $.local_variable_declaration, $.multi_assignment_declaration],
    // §5.5 — a literal after a command chain's comma is a positional
    // argument, or the start of a trailing method_invocation's function
    // expression; only `(` would make it the latter.
    [$._command_argument, $._expression],
    // `foo bar, baz` is a call; `foo bar` alone is two statements. The
    // comma decides, so both readings stay alive until then.
    [$.command_chain, $._expression],
    [$.command_chain, $._expression, $._type],
    // §5.5 — `foo [:], m` (a command's list or map argument) or `foo[k]`.
    [$.command_chain, $._expression, $._array_type_value, $._type],
    [$.command_chain, $._type],
    // §5.5 — `(el x)` is a command call; `(String s) -> …` a lambda's
    // parameter list. The `->` decides.
    [$._type, $._parenthesized_command],
    // `(f(a), b)` is a command call, `(f(a))` a call: the comma decides.
    [$._expression, $._parenthesized_command],
    // In an enum constant's arguments, `E(A b` or `E(A b(1))`, also a constructor's
    // parameters; `f(g h(1))` is `f(g(h(1)))`.
    [$._type, $._argument_list_command, $._argument_command],
    // `a(x) or(y)`: a call chained onto a call, or a call then a new name.
    [$._call_chain, $._expression],
    // `[a][` may index the list, or open an empty index `[a][]`.
    [$._expression, $._empty_list_subscript],
    // See SPECIFICATION.md §7. Populated as rules accumulate.
    // (list_literal vs map_literal — tree-sitter resolves without
    //  an explicit conflict because `:` after the first element
    //  always disambiguates within one token of lookahead.)
    //
    // §4 — inside enum_body, right after `{`, a bare identifier could
    // still be starting an enum_constant (`RED`) or could be a
    // field_declaration's type (`String dataType`) now that the
    // member section no longer requires a leading `;` to separate it
    // from the constant list. The next identifier vs `,`/`(`/`}`
    // disambiguates.
    [$.enum_constant, $._type],

    // §4 — `enum Type { NAME(x, ...` — inside the parens, a bare
    // identifier could still be an enum_constant's constructor
    // argument (`_expression`) or the start of the *next* member's
    // constructor/method formal_parameter, once the member section
    // can directly follow the constant list. Resolved by what comes
    // after: `,`/`)` (argument) vs a second identifier (typed
    // parameter) vs nothing (untyped parameter name).
    [$.formal_parameter, $._expression],

    // §5.15 — inside `( … )` an identifier could be either an
    // _expression (parenthesized_expression) or a _type
    // (parenthesized_type_cast). Tree-sitter explores both via
    // this conflict; the `)` plus the next token disambiguates.
    [$._expression, $._type],
    // `(@A String)` is a cast's annotated type or a lambda's parameter.
    [$.formal_parameter, $._type],

    // §4 — `a.b.c` could be a field_access chain (expression
    // context) or a qualified_type (type context). Same family
    // as the _expression / _type conflict above but the implicit
    // qualified_type_repeat1 aux symbol forces a separate
    // declaration.
    [$._expression, $.qualified_type],

    // §3.2.2 — `_cast_type` overlaps `_type` on identifier / array /
    // generic alternatives; only the dotted path diverges. The
    // companion three-way conflict pulls in `_dotted_type` so the
    // dotted shape can win the trailing `.identifier` from the
    // surrounding `field_access` reduction.
    [$._cast_type, $._type],
    [$._cast_type, $._dotted_type, $.qualified_type],
    // §5.14 — `(List<X>)` is a cast's type or a parenthesized generic value.
    [$.generic_type, $._generic_type_value],

    // §4 — `for ( identifier …` could continue as a for_statement
    // (identifier is part of the init expression) or as a
    // for_in_statement (identifier is the loop variable). The
    // token after the identifier (`;` / operator vs `in`)
    // disambiguates.
    [$.for_in_statement, $._expression],
    [$.for_in_statement, $._expression, $._type],
    [$.for_in_statement, $._type],


    // §4 — a record takes only some modifiers; until `record` the
    // modifiers may belong to any declaration.
    [$._modifier, $.record_declaration],
    [$._modifier, $.record_declaration, $.local_variable_declaration],

    // §5.2 — `def identifier` could either be a
    // local_variable_declaration (variable_declarator with no
    // initializer) or the start of a method_declaration
    // (_property_name + parameters). The `(` after the name
    // decides; the conflict keeps both alternatives alive until
    // then.
    [$.variable_declarator, $._property_name],

    // §5.2 — a map key may be a statement keyword (`[class: x]`); the
    // reserved-free key path and the expression path merge at the `:`.
    [$._annotation_type_element, $._method_name],
    [$.variable_declarator, $._method_name],
    [$._expression, $.map_entry],

    // §4 — `static NAME` inside a class body: after one or more
    // modifiers and one identifier, that identifier could be the
    // modifier-only field_declaration's variable_declarator name
    // (`static MEDIUM_TTL = 900`) or the start of a typed
    // field_declaration/method_declaration's `_type` (`static Foo
    // bar`). The next token (`=`/`,`/`;` vs a second identifier)
    // disambiguates.
    //
    // §4 — `@Field NAME` at statement level: after the annotations, the
    // identifier is the declarator name (`@Field NAME = 1`) or the type
    // (`@Field String NAME`). The next token decides.
    //
    // §4 — `@Shared NAME` in a class body: after the annotations, the
    // identifier is the declarator name (`@Shared log = []`) or the type
    // (`@Shared Sql sql`). The next token decides.
    [$.variable_declarator, $._type],

    // §5.9 — inside annotation arguments, `name = …` and `[ … ]` may hold
    // annotations; until one appears they read as ordinary expressions.
    [$._annotation_element_pair, $._expression],
    [$._annotation_list, $.list_literal],
    [$._annotation_list_element, $.list_literal],

    // §5.8 — `(a, b) -> …` and `x -> …` share a prefix with parenthesized
    // expressions, casts and plain identifiers until the arrow decides.
    [$.formal_parameter, $._multi_assignment_names],
    [$.formal_parameter, $._expression, $._type],

    // §4 — an annotation type element `String value() default 'x'`.
    [$._annotation_type_element, $._property_name],

    // §4 — `String[]` is a type, or a value (`String[].class`); after the
    // name a subscript is possible too, until `]` follows `[`.
    [$._expression, $._array_type_value, $._type],
    [$._expression, $._array_type_value],
    [$._array_type_value, $._type],
    // `{ a [], b }`: a closure's command `a([], b)`, or an array initializer
    // whose first value is the type `a[]`; the `}` or `,` path decides.
    [$._command_list_argument, $._array_type_value],

    // §5.5 — `assert x = cmd a, 'k': v`: a quoted named argument, or the
    // assert's message after `:`.
    [$._command_quoted_named_argument, $._expression],

    // §5.8 — `{ a = 1` is a closure parameter with a default value or a
    // closure body's assignment; `->` decides.
    [$._expression, $._closure_parameter_name_default, $.closure_parameter],
    [$._value, $.closure_parameter],
    // `{ File f = d() -> }`: a closure parameter's default call, or a declaration's
    // lambda command, `{ File f = d () -> 1 }`; prec.dynamic picks the parameter.
    [$._command_lambda_parameters, $._closure_default_arguments],

    // §5.10 — after `{` and a `_line_break`, a closure's parameters or its
    // first statement may follow; in an enum, a constant or a member.
    [$.closure],
    [$.block, $.closure],
    [$._new_closure_on_line],
    [$.enum_body],
    // §4 — in an enum, `E() {` starts a constant with a body or a
    // constructor; a `,` after the body makes it a constant.
    [$.formal_parameters, $.argument_list],
    // §4 — `@A X` in an enum: an annotated constant, or an annotated field.
    [$.enum_constant, $.variable_declarator],
    [$.enum_constant, $.variable_declarator, $._type],

    // §4 — `List<String> x` declares `x` with no value; `List<String> x = 1`
    // is the initialized form. The token after the name decides.
    [$.local_variable_declaration, $._type],

    // `new X()` then `{` on the next line: an anonymous class body or a
    // closure argument; the body that parses decides (see object_creation_expression).
    [$.object_creation_expression],
  ],

  rules: {
    // Top-level. Real rule set lands incrementally per
    // SPECIFICATION.md §4 (statement and declaration coverage).
    source_file: $ => seq(
      repeat($.shebang),
      repeat(choice($._statement, ';', $._line_break)),
    ),

    // §4 — `#!/usr/bin/env groovy` line at file start. Single
    // token consumed by the in-grammar lexer.
    // Several may open the file (nix-shell: `#! nix-shell -i groovy`).
    shebang: _ => token(seq('#!', /[^\n]*/)),

    // §4 — statement-level alternatives. Declarations and the rest
    // of control flow fill out this choice in subsequent iterations.
    // `block` is intentionally NOT here — a top-level `{ ... }`
    // continues to parse as an expression_statement-wrapped closure
    // (the historically intended form for Groovy scripts). Blocks
    // appear explicitly in the body slot of `if` / `while` / etc.
    _statement: $ => choice(
      $.expression_statement,
      $.if_statement,
      $.while_statement,
      $.synchronized_statement,
      $.do_while_statement,
      $.for_statement,
      $.for_in_statement,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.throw_statement,
      $.assert_statement,
      $.yield_statement,
      $.try_statement,
      $.class_declaration,
      $.trait_declaration,
      $.interface_declaration,
      $.annotation_type_declaration,
      $.enum_declaration,
      $.record_declaration,
      $.package_declaration,
      $.import_declaration,
      $.local_variable_declaration,
      $.multi_assignment_declaration,
      $.pipeline_statement,
      $.method_declaration,
      // A script method with modifiers and no return type: `private f(x) { }`.
      // In a class body the same shape is a constructor.
      alias($._script_untyped_method, $.method_declaration),
      $.labeled_statement,
      $.command_chain,
    ),

    // prec: after modifiers, a name and `(` start this, never a declarator.
    // An annotation stands in for a modifier: `@CompileStatic foo(x) { }`.
    _script_untyped_method: $ => prec(1, seq(
      choice(
        seq(repeat($.annotation), repeat1($._modifier)),
        repeat1($.annotation),
      ),
      field('name', $.identifier),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
      field('body', $.block),
    )),

    // §5.5 — command_chain is the parenthesis-free method-call
    // form (`println 'hello'`, `events 'a', 'b', 'c'`). v1 is
    // intentionally conservative: receiver is a bare identifier
    // and arguments are restricted to literal-shaped expressions
    // and closures (no risk of mis-parsing `foo bar` as a command
    // call on a bare identifier). Multi-argument lists are supported
    // (comma-separated literal arguments per Gradle / Jenkins DSL
    // idioms). Chained `foo bar baz` and complex argument shapes
    // land later. Closes a subset of murtaza64 #5.
    //
    // The receiver may also be a dotted path (`script.writeFile file: f`,
    // `response.setHeader 'k', v`). A first argument that starts with a
    // string or a number may be any expression (`log.warn '*' * 80`,
    // `myActor 10 as BigDecimal`); the zero-width `_command_literal_start`
    // only matches when it starts on the receiver's line. A dotted receiver
    // takes no closure, since `list.each { … }` is a method_invocation. A
    // closure on a bare receiver's line may likewise lead a longer expression
    // (`thrownException { f() } instanceof IllegalStateException`). An
    // identifier, dotted or call-shaped first argument needs a comma after
    // it (`bind Foo, Bar`), because `foo bar` alone is two statements or a
    // declaration. That last form carries no precedence, so the parser
    // keeps both readings until the comma (or its absence) decides.
    command_chain: $ => choice(
      prec(2, seq(
        field('receiver', $.identifier),
        choice(
          seq($._command_closure_start, field('argument', $._expression)),
          field('argument', $._command_argument),
        ),
        repeat($._command_later_argument),
      )),
      prec(2, seq(
        // also `session?.setAttribute k, v` and `all*.exclude group: 'g'`,
        // and a quoted name as builder and ruleset DSLs write it:
        // `'x:title' lang: 'en', 'text'`, `'CatchThrowable' priority: 1`.
        field('receiver', choice(
          $.identifier,
          $.field_access,
          $.safe_navigation_expression,
          $.spread_dot_expression,
          $.string_literal,
        )),
        choice(
          seq($._command_literal_start, field('argument', choice(
            $._expression,
            alias($._command_quoted_named_argument, $.named_argument),
          ))),
          field('argument', alias($._command_named_argument, $.named_argument)),
        ),
        repeat($._command_later_argument),
      )),
      seq(
        // also `resolver?.setLocale request, response, l`
        field('receiver', choice(
          $.identifier,
          $.field_access,
          $.safe_navigation_expression,
          $.spread_dot_expression,
        )),
        field('argument', $._command_trailing_argument),
        repeat1($._command_later_argument),
      ),
      // A parenthesized receiver: `((int) 1 / 2) 1, 2` is
      // `((int) 1 / 2).call(1, 2)`. Where the parentheses could hold a
      // type, `(a) 1`, the cast wins.
      prec.dynamic(-1, seq(
        field('receiver', $.parenthesized_expression),
        $._command_literal_start,
        field('argument', $._expression),
        repeat($._command_later_argument),
      )),
      // A quoted or interpolated name: `"${op}" p, 2`.
      seq(
        field('receiver', $.string_literal),
        field('argument', choice($.identifier, $.field_access, $.method_invocation)),
        repeat1($._command_later_argument),
      ),
      // A call chained onto a call on the same line, as Groovy's command
      // chains allow: `isOrIsCausedBy(e, A) or(isOrIsCausedBy(e, B))`.
      $._call_chain,
      $._lambda_command,
      // `println as`, `println in`: a variable named `as` or `in` (Groovy 3
      // and 4) as the one argument, with the statement ending after it.
      seq(
        field('receiver', $.identifier),
        field('argument', alias($._command_keyword_argument, $.identifier)),
      ),
      // `if (c) a.b key` then `else`: a name alone after a dotted receiver,
      // with the statement ending after it.
      seq(
        field('receiver', $.field_access),
        $._command_name,
      ),
    ),

    // Also an argument: `f(a(x) or(b(y)))`.
    _call_chain: $ => prec.right(seq(
      field('receiver', $.method_invocation),
      repeat1(field('argument', alias($._name_call, $.method_invocation))),
    )),

    // Also a longer chain of names, `foo b bar c` (`foo(b).bar(c)`), which
    // elsewhere is read as separate statements; the `else` decides.
    _bare_name_command: $ => seq(
      field('receiver', $.identifier),
      repeat(prec.dynamic(-1, seq(field('argument', $.identifier), $.identifier))),
      $._command_name,
    ),

    // `if (c) render result as JSON` then `else`, Grails' idiom: a bare
    // receiver's one cast argument. Elsewhere the line reads as two
    // statements, `render` and `result as JSON`; prec.dynamic keeps that
    // reading wherever both parse, so only the `else` decides.
    _bare_cast_command: $ => prec.dynamic(-1, seq(
      field('receiver', $.identifier),
      field('argument', alias($._bare_cast_argument, $.cast_expression)),
    )),

    // `if (c) o.f a - b` then `else` on the next line: an operator expression as
    // the one argument, `o.f(a - b)` (Groovy 2.4, 3 and 4), which elsewhere reads
    // as two statements. The scanner gives its first name as `_command_operand`
    // only when an `else` starts the next line.
    _bare_operator_command: $ => seq(
      field('receiver', choice($.identifier, $.field_access)),
      field('argument', alias($._operand_binary, $.binary_expression)),
    ),

    _operand_binary: $ => binaryOperators($, $._expression, choice(
      alias($._command_operand, $.identifier),
      alias($._operand_binary, $.binary_expression),
    )),

    _bare_cast_argument: $ => seq(
      field('value', $.identifier),
      'as',
      field('type', $._cast_type),
    ),

    _command_name: $ => choice(
      field('argument', alias($._command_name_argument, $.identifier)),
      seq($._command_name_start, field('argument', $.identifier)),
    ),

    // After the first argument has made the statement a command chain,
    // a later argument is any expression or a named argument:
    // `helper.registerAllowedMethod 'sh', [Map], { m -> }`.
    // A later argument may be spread, `commandLine 'gradlew', *args`.
    _command_later_argument: $ => seq(',', field('argument', choice(
      $._expression,
      alias($._command_named_argument, $.named_argument),
      alias($._command_quoted_named_argument, $.named_argument),
      $.spread_arguments,
    ))),

    // §5.5 item 3 — named arguments in a parenthesis-free call:
    // `exclude group: 'a', module: 'b'`, `archiveArtifacts artifacts: 'x'`,
    // GORM's `name blank: false, size: 1..100`. The key is an identifier
    // lexed by the external scanner only when `:` follows on the same
    // line, so `Type name = …` and a bare statement followed by a Spock
    // label on the next line are unaffected.
    _command_named_argument: $ => seq(
      field('key', alias($._command_named_key, $.identifier)),
      ':',
      field('value', $._expression),
    ),

    // A quoted key works too, `attributes 'Main-Class': main`, where a
    // string may start an argument: after `_command_literal_start` or a comma.
    // So does a number, `select 1: -1`, `select 1.5: 2, 3: 4`.
    _command_quoted_named_argument: $ => seq(
      field('key', choice($.string_literal, $.number_literal)),
      ':',
      field('value', $._expression),
    ),

    // §5.5 item 3 — after the first argument has made the statement a
    // command chain, later positional arguments may also be identifiers,
    // dotted paths, method calls and subscripts:
    // `systemProperty 'k', System.getProperty('k')`, `assertEquals 'v', params['k']`.
    // The first argument stays restricted, since `foo bar` with no comma
    // is a typed declaration.
    _command_trailing_argument: $ => choice(
      $.identifier,
      $.field_access,
      $.method_invocation,
      $.subscript_expression,
      // `areEqualSets games*.name, [a]`, `foo q.@b, 1`, `foo a.&b, 1`, `foo a::b, 1`
      $.spread_dot_expression,
      $.direct_field_access_expression,
      $.method_pointer_expression,
      $.method_reference_expression,
      $.safe_chain_dot_expression,
      $.safe_subscript_expression,
      // `shouldFail foo + 'x', msg`, `assertTrue x instanceof Y, msg`,
      // `json.authors authors as Author[], { }`
      $.binary_expression,
      $.instanceof_expression,
      $.cast_expression,
      // `menuIcon ext == 'pdf' ? A : B, m`, `exec python ?: 'python3', 'x.py'`
      $.ternary_expression,
      $.elvis_expression,
      // `assertEquals [:], map`: a list or map whose `[` is on the
      // receiver's line, which the comma after it tells from a subscript.
      alias($._command_list_argument, $.list_literal),
      alias($._command_map_argument, $.map_literal),
      // `assertEquals (-1), x`: a parenthesized first argument, which the
      // comma after it tells from an argument list.
      alias($._command_parenthesized_argument, $.parenthesized_expression),
    ),

    _command_parenthesized_argument: $ => seq(
      alias($._call_open, '('),
      $._expression,
      ')',
    ),

    _command_list_argument: $ => seq(
      alias($._subscript_open, '['),
      optional(seq(
        choice($._expression, $.spread_arguments),
        repeat(seq(',', repeat(','), choice($._expression, $.spread_arguments))),
        optional(','),
      )),
      ']',
    ),

    _command_map_argument: $ => {
      const entry = choice($.map_entry, $.spread_map_entry);
      return seq(
        alias($._subscript_open, '['),
        choice(':', seq(entry, repeat(seq(',', repeat(','), entry)), optional(','))),
        ']',
      );
    },

    _command_argument: $ => choice(
      $.boolean_literal,
      $.null_literal,
      $.closure,
    ),

    // §5.3 — `label: stmt`. Required only at statement-start
    // position; map_entries (which share `:`) are inside `[…]`
    // and don't collide here. `prec(1, ...)` keeps both
    // labeled_statement and expression_statement alive after
    // `identifier`; the `:` decides. A scanner-token form
    // (`_label_colon`) was attempted but interacted with
    // named-argument command chains (`archiveArtifacts artifacts: 'x'`),
    // which now lex their key through `_command_named_key` instead;
    // see divergence #5.
    labeled_statement: $ => prec(1, seq(
      field('label', $.identifier),
      ':',
      field('statement', $._statement),
    )),

    // §4 — block of statements, used in control-flow bodies and
    // (eventually) method bodies. Shares `{ ... }` syntax with
    // closure (§5.8). A `prec(1, …)` keeps block preferred at use
    // sites where both `block` and a closure-bearing `_statement`
    // are valid alternatives (e.g. the body slot of `if` / `while`).
    // One command call as the whole body, as for a closure (see closure):
    // `for (k in m) { println k }`.
    block: $ => prec(1, seq(
      '{',
      choice(
        repeat(choice($._statement, ';', $._line_break)),
        alias($._closure_command, $.command_chain),
        alias($._closure_declaration, $.local_variable_declaration),
      ),
      '}',
    )),

    // §4 — if / else. `prec.right` resolves the dangling-else
    // ambiguity in favour of the closest `if`. Body accepts either
    // an explicit block (preferred) or a single statement. The
    // `optional(';')` after the consequence keeps the else attached
    // when a braceless consequence is terminated by a semicolon
    // (`if (x) a(); else b()`); without it the `;` is claimed by the
    // enclosing statement list (§5.10) and the `else` is severed. It
    // sits outside the else clause so the no-else form (`if (x) a();
    // b()`) does not require an `else` after eating the `;`.
    if_statement: $ => prec.right(seq(
      'if',
      '(',
      // also a command call: `if (msg.contains 'x') { … }`
      field('condition', choice(
        $._expression,
        alias($._parenthesized_command, $.command_chain),
      )),
      ')',
      // `if (follower) follower value` then `else` on the next line: a bare
      // receiver's one name argument, which elsewhere would be two statements.
      field('consequence', choice(
        $.block,
        $._statement,
        alias($._bare_name_command, $.command_chain),
        alias($._bare_cast_command, $.command_chain),
        alias($._bare_operator_command, $.command_chain),
      )),
      optional(';'),
      optional(seq(
        'else',
        field('alternative', choice($.block, $._statement)),
      )),
    )),

    // §4 — while loop. do-while lands separately.
    while_statement: $ => seq(
      'while',
      '(',
      // also a command call (Groovy 3 and 4): `while (a.b c)`
      field('condition', choice(
        $._expression,
        alias($._parenthesized_command, $.command_chain),
      )),
      ')',
      field('body', choice($.block, $._statement, ';')),
    ),

    // §4 — `synchronized (lock) { … }`, the Java block form.
    synchronized_statement: $ => seq(
      'synchronized',
      '(',
      field('lock', $._expression),
      ')',
      field('body', $.block),
    ),

    // §4 — C-style for. init / condition / update slots all
    // optional; bare `for (;;)` is the infinite-loop idiom. The init
    // slot accepts a single expression (`i = 0`) or a variable
    // declaration (`int i = 0`, `def i = 0`, incl. multiple
    // declarators). Apache `GroovyParser.g4` `forInit:
    // localVariableDeclaration | expressionList` also allows a
    // comma-separated expression list (`i = 0, j = 1`), and the update
    // slot takes one too (`i++, j++`), as in Groovy 3 and 4.
    for_statement: $ => seq(
      'for',
      '(',
      optional(choice(
        seq(field('init', $._expression), repeat(seq(',', field('init', $._expression)))),
        field('init', $.local_variable_declaration),
        // `for (def (int i, int j) = [0, 0]; i < 5; i++)` (Groovy 3+)
        field('init', alias($._for_multi_assignment, $.multi_assignment_declaration)),
        // `for (int i; i < 3; i++)`: a typed declaration without a value.
        field('init', alias($._for_bare_declaration, $.local_variable_declaration)),
      )),
      ';',
      optional(field('condition', $._expression)),
      ';',
      optional(seq(
        field('update', $._expression),
        repeat(seq(',', field('update', $._expression))),
      )),
      ')',
      // also an empty body: `for (i = 0; i < 5; i++);`
      field('body', choice($.block, $._statement, ';')),
    ),

    // `def` only: `final` and annotations there would compete with a for-in
    // loop's modifiers.
    _for_multi_assignment: $ => seq(
      'def',
      '(',
      $._multi_assignment_target,
      repeat(seq(',', $._multi_assignment_target)),
      ')',
      '=',
      field('value', $._expression),
    ),

    _for_bare_declaration: $ => seq(
      optional('final'),
      field('type', $._type),
      alias($._bare_declarator, $.variable_declarator),
      repeat(seq(',', $.variable_declarator)),
    ),

    // §4 — do-while. Body is required; condition follows the body.
    // `optional(';')` lets a braceless body be terminated before
    // `while` (`do a(); while (c)`, the Java/Groovy `do S while (e);`
    // form); without it the `;` errors since `while` is required next.
    do_while_statement: $ => seq(
      'do',
      field('body', choice($.block, $._statement)),
      optional(';'),
      'while',
      '(',
      field('condition', $._expression),
      ')',
    ),

    // §4 — return / break / continue / throw / assert. Until
    // AUTOMATIC_SEMICOLON lands the parser will greedily consume
    // a trailing expression onto `return` / `throw` / `assert`;
    // explicit `;` or statement boundary is recommended in source.
    return_statement: $ => prec.right(seq(
      'return',
      optional($._expression),
    )),

    break_statement: $ => prec.right(seq(
      'break',
      optional(field('label', $.identifier)),
    )),

    continue_statement: $ => prec.right(seq(
      'continue',
      optional(field('label', $.identifier)),
    )),

    throw_statement: $ => seq('throw', $._expression),

    // §4 — `yield expr` in a Groovy 4 switch expression's case
    // body. `yield` is a contextual keyword (§5.2) so it remains
    // a usable identifier elsewhere — tree-sitter's word:
    // directive resolves the ambiguity.
    yield_statement: $ => prec.right(seq(
      alias($._yield_keyword, 'yield'),
      $._expression,
    )),

    // §4 — power-assert: `assert e`, `assert e : msg` or, as in Java,
    // `assert e, msg`. The colon separator may collide with map_entry in
    // later iterations, but assert is a statement-only construct so the
    // context resolves it.
    assert_statement: $ => prec.right(seq(
      'assert',
      $._expression,
      optional(seq(choice(':', alias($._expression_colon, ':'), ','), field('message', $._expression))),
    )),

    // §4 — try / catch / finally. At least one of catch/finally is
    // required per Java/Groovy semantics; the grammar relaxes that
    // to allow a bare `try { … }` and lets downstream tooling
    // surface the error if needed. Try-with-resources lands later
    // with local variable declarations.
    try_statement: $ => seq(
      'try',
      optional(field('resources', $.resource_specification)),
      field('body', $.block),
      repeat($.catch_clause),
      optional($.finally_clause),
    ),

    // §4 — `try (Foo f = expr; Bar b = expr) { … }`. Resources
    // are typed or `def`-style declarators with required `=`
    // initializer (per Java-style try-with-resources). Multiple
    // resources separated by `;`.
    resource_specification: $ => seq(
      '(',
      $.resource,
      repeat(seq(choice(';', $._line_break), $.resource)),
      optional(';'),
      ')',
    ),

    // An existing variable is a resource too, as in Java 9: `try (parser)`.
    resource: $ => choice(
      seq(
        optional('final'),
        optional(field('type', $._type)),
        field('name', $.identifier),
        '=',
        field('value', $._expression),
      ),
      field('name', $.identifier),
    ),

    catch_clause: $ => seq(
      'catch',
      '(',
      field('parameter', $.catch_formal_parameter),
      ')',
      field('body', $.block),
    ),

    // §4 — `catch_formal_parameter` accepts either a single _type
    // or a multi_type for Java 7+ / Groovy multi-catch. Closes
    // murtaza64 #39. The type itself is optional — Groovy allows a
    // bare `catch (e) { … }`, implicitly typed as Exception.
    // Annotations may come before or after `final` (Groovy 3 and 4):
    // `catch (@A Exception e)`.
    catch_formal_parameter: $ => seq(
      repeat($.annotation),
      optional(seq('final', repeat($.annotation))),
      optional(field('type', choice($._type, $.multi_type))),
      field('name', $.identifier),
    ),

    multi_type: $ => seq(
      $._type,
      repeat1(seq('|', $._type)),
    ),

    finally_clause: $ => seq(
      'finally',
      field('body', $.block),
    ),

    // §4 — class / trait / interface declarations. v1 supports the
    // shape `[keyword] Name { body }`. extends, implements, generics,
    // modifiers, and sealed / permits clauses land in later
    // iterations. trait closes the dekobon #247 trait row.
    // §4 — class declaration with optional Groovy 4 sealed modifier
    // and `permits` clause. The `non-sealed` form is a single
    // anonymous token (token('non-sealed')) so the dash doesn't
    // get lexed as subtraction. Modifiers and annotations may follow it
    // (Groovy 4): `sealed abstract class W`, `sealed @Deprecated class W`.
    // `def` may lead the modifiers: `def class X`, `def static class X`.
    // Annotations may also follow modifiers: `static @Immutable class X`.
    class_declaration: $ => seq(
      repeat($.annotation),
      optional('def'),
      optional(seq(repeat1($._modifier), repeat1($.annotation))),
      repeat($._modifier),
      optional(seq(
        choice(alias($._sealed_keyword, 'sealed'), token('non-sealed')),
        repeat(choice($._sealed_modifier, $.annotation)),
      )),
      'class',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.superclass),
      optional($.super_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    trait_declaration: $ => seq(
      repeat($.annotation),
      repeat($._modifier),
      optional(seq(
        choice(alias($._sealed_keyword, 'sealed'), token('non-sealed')),
        repeat(choice($._sealed_modifier, $.annotation)),
      )),
      alias($._trait_keyword, 'trait'),
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.extends_interfaces),
      // a trait may implement interfaces, like a class
      optional($.super_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    interface_declaration: $ => seq(
      ...memberModifiers($),
      optional('def'),
      optional(seq(
        choice(alias($._sealed_keyword, 'sealed'), token('non-sealed')),
        repeat(choice($._sealed_modifier, $.annotation)),
      )),
      'interface',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.extends_interfaces),
      optional($.permits_clause),
      field('body', alias($._interface_body, $.class_body)),
    ),

    // An interface method may have no return type (Groovy 3 and 4): `key()`.
    _interface_body: $ => seq(
      '{',
      repeat(choice(
        $._class_member,
        alias($._untyped_abstract_method, $.method_declaration),
        $.instance_initializer,
        ';',
        $._line_break,
      )),
      '}',
    ),

    _untyped_abstract_method: $ => seq(
      field('name', $.identifier),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
    ),

    // §4 / §5.14 — generic type parameter list on a class /
    // interface / trait / method declaration. After the declared name
    // a `<` can only open type parameters, so a space may come before it.
    type_parameters: $ => seq(
      '<',
      $.type_parameter,
      repeat(seq(',', $.type_parameter)),
      '>',
    ),

    // A bound may carry type-use annotations: `<T extends @A CharSequence>`.
    type_parameter: $ => seq(
      field('name', alias($.identifier, $.type_identifier)),
      optional(seq(
        'extends',
        repeat($.annotation),
        $._type,
        repeat(seq('&', repeat($.annotation), $._type)),
      )),
    ),

    // §4 — class has a single super and zero-or-more interfaces.
    superclass: $ => seq('extends', repeat($.annotation), $._type),

    // A type-use annotation may come before each: `implements @A Consumer<T>`.
    super_interfaces: $ => seq(
      'implements',
      repeat($.annotation),
      $._type,
      repeat(seq(',', repeat($.annotation), $._type)),
    ),

    // §4 — trait / interface extend multiple interfaces (no
    // `implements` keyword in that position).
    extends_interfaces: $ => seq(
      'extends',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — Java-style modifiers. Each one is a hard reserved
    // keyword (§5.2) so the `word: $.identifier` directive keeps
    // them from accidentally being treated as identifiers.
    _modifier: _ => choice(
      'public',
      'private',
      'protected',
      'static',
      'final',
      'abstract',
      'synchronized',
      'native',
      'transient',
      'volatile',
      'strictfp',
    ),

    // Not `final`, which Groovy rejects after `sealed` or `non-sealed`
    // (`sealed final interface I`).
    _sealed_modifier: _ => choice(
      'public',
      'private',
      'protected',
      'static',
      'abstract',
      'synchronized',
      'native',
      'transient',
      'volatile',
      'strictfp',
    ),

    permits_clause: $ => seq(
      'permits',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — `@interface Foo { … }` declares an annotation type.
    // `@` followed by the `interface` keyword is unambiguous —
    // `interface` is a hard reserved keyword (§5.2) so the
    // alternative parse `annotation` (which needs identifier)
    // cannot fire here.
    annotation_type_declaration: $ => seq(
      repeat($.annotation),
      repeat($._modifier),
      '@',
      'interface',
      field('name', $.identifier),
      field('body', alias($._annotation_type_body, $.class_body)),
    ),

    // An annotation type's body also takes elements with a default value:
    // `String value() default 'x'`.
    _annotation_type_body: $ => seq(
      '{',
      repeat(choice(
        $._class_member,
        alias($._annotation_type_element, $.method_declaration),
        ';',
        $._line_break,
      )),
      '}',
    ),

    _annotation_type_element: $ => seq(
      repeat($.annotation),
      repeat($._modifier),
      field('return_type', $._type),
      // also a quoted name (Groovy 3 and 4): `String[] 'default'() default []`
      field('name', choice($.identifier, alias($.string_literal, $.quoted_identifier))),
      field('parameters', alias($._empty_parameters, $.formal_parameters)),
      'default',
      field('default', choice(
        $._expression,
        $.annotation,
        alias($._annotation_list, $.list_literal),
      )),
    ),

    _empty_parameters: $ => seq(choice('(', alias($._call_open, '(')), ')'),

    // §4 — class members: methods, fields, constructors, static
    // initialisers, and nested type declarations.
    // The body accepts `;` as an empty member / terminator the same
    // way statement lists do (§5.10): a field or abstract method is
    // commonly written `int x = 1;` / `void f();`, and a stray `;` is
    // a no-op member. `;` is anonymous, so it never appears in the AST.
    class_body: $ => seq(
      '{',
      repeat(choice($._class_member, $.instance_initializer, ';', $._line_break)),
      '}',
    ),

    // Nested types are members too: a nested `enum` (`static enum
    // EventType { … }` inside a domain class), `static class Config { … }`,
    // or a member `interface`, `trait`, `record` or `@interface`. Each
    // starts with its own keyword after the shared annotations and
    // modifiers, so the keyword decides.
    _class_member: $ => choice(
      $.method_declaration,
      alias($._bodiless_untyped_method, $.method_declaration),
      alias($._default_method, $.method_declaration),
      $.field_declaration,
      $.constructor_declaration,
      $.static_initializer,
      $.enum_declaration,
      $.class_declaration,
      $.interface_declaration,
      $.trait_declaration,
      $.record_declaration,
      $.annotation_type_declaration,
    ),

    // §4 — constructors share the method_declaration shape but have
    // no return type and the name matches the enclosing class. We
    // don't enforce the name match (downstream tooling can), but we
    // emit a distinct node kind so IDE features (rename, jump) can
    // tell constructors apart from methods. `prec(3, …)` wins over
    // both field and method when the identifier is followed by `(`
    // and no return type precedes it.
    constructor_declaration: $ => prec.right(3, seq(
      ...memberModifiers($),
      field('name', $.identifier),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
      field('body', $.block),
    )),

    // §4 — `static { … }` class-body initializer block. Runs once
    // when the class is loaded. Distinct node kind so downstream
    // tools can index it.
    static_initializer: $ => seq(
      'static',
      field('body', $.block),
    ),

    // §4 — `{ … }` as a class member runs for each new instance; the
    // "double brace" idiom `new HashMap() {{ put('a', 1) }}` is one.
    instance_initializer: $ => field('body', $.block),

    // §4 — class / interface / trait field. Java-style shape: optional
    // annotations, optional modifiers, `def` or a `_type`, then one or
    // more declarators (each with optional initializer). Method
    // declarations share the same prefix; `prec(2, …)` on
    // `method_declaration` and the trailing `(` discriminate.
    //
    // A modifier alone (no `def`, no type) also introduces a field —
    // `static MEDIUM_TTL = 900` infers Object, a common constant-field
    // idiom. Gated behind `repeat1($._modifier)` (not `repeat`) so this
    // alternative can't match a bare, modifier-less identifier, which
    // would collide with a plain expression_statement.
    //
    // An annotated field may omit the type, as Spock's `@Shared log = []`
    // does; the annotation marks it as a field declaration.
    //
    // `def static NAME = …` puts the modifiers after `def`.
    field_declaration: $ => choice(
      seq(
        ...memberModifiers($),
        choice(
          // `def @Shared String x`: annotations may follow `def` too.
          seq('def', ...defModifiers($)),
          seq('def', ...defModifiers($), field('type', $._type)),
          field('type', $._type),
        ),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      // prec.dynamic: `private boolean isLeaf() { }` must stay one method,
      // not an untyped field `boolean` followed by a constructor `isLeaf`.
      // Both readings reach the `}`; without it the merge picks one by tree
      // shape, which a comment before the member can flip.
      prec.dynamic(-1, seq(
        repeat($.annotation),
        repeat1($._modifier),
        optional(seq(repeat1($.annotation), repeat($._modifier))),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      )),
      // prec.dynamic: `@Inject Engine engine` must stay one typed field,
      // not an untyped field `Engine` followed by one more member.
      prec.dynamic(-1, seq(
        repeat1($.annotation),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      )),
    ),

    // §4 — `def name(params) body`. Typed return type (`String foo()
    // { … }`) lands once we can disambiguate from local variable
    // declaration. Modifiers and throws clause similarly defer.
    // prec(1) wins over local_variable_declaration when `(` follows
    // the name, so `def f() { … }` at script-top-level parses as a
    // method rather than a `def f` variable followed by `(…) { … }`.
    //
    // Groovy also takes modifiers or type parameters after `def`
    // (`def static foo()`, `def <T> T foo()`) and a bodiless method with
    // modifiers but no return type (`abstract setVersion(v)`).
    //
    // A modifier stands in for `def` before a quoted name, as in a Spock
    // feature method `private "a feature"() { … }`.
    //
    // A return type on the line before the name is a separate path that
    // loses to any other reading (prec.dynamic): `a.b` and then `bar(z) { }`
    // on the next line are two statements. `_line_break` after the type
    // only matches across a line break.
    method_declaration: $ => prec.right(2, choice(
      seq(
        ...memberModifiers($),
        choice(
          seq(
            optional(field('type_parameters', $.method_type_parameters)),
            choice(
              seq('def', repeat($._modifier), optional(field('return_type', $._type))),
              field('return_type', $._type),
            ),
            methodNameToBody($),
          ),
          seq(
            'def',
            field('type_parameters', $.method_type_parameters),
            field('return_type', $._type),
            methodNameToBody($),
          ),
        ),
      ),
      prec.dynamic(-3, seq(
        ...memberModifiers($),
        optional(field('type_parameters', $.method_type_parameters)),
        field('return_type', $._type),
        $._line_break,
        methodNameToBody($),
      )),
      // An annotation stands in for a modifier too: `@Unroll "a feature"() { }`.
      seq(
        choice(
          seq(repeat($.annotation), repeat1($._modifier)),
          repeat1($.annotation),
        ),
        field('name', alias($.string_literal, $.quoted_identifier)),
        field('parameters', $.formal_parameters),
        optional($.throws_clause),
        field('body', $.block),
      ),
    )),

    // An interface or trait's `default` method, a class member only:
    // `default Class<?> type() { Object }`.
    _default_method: $ => prec.right(seq(
      ...memberModifiers($),
      'default',
      repeat($._modifier),
      optional(field('type_parameters', $.method_type_parameters)),
      optional(field('return_type', $._type)),
      methodNameToBody($),
    )),

    // `abstract setVersion(version)`: modifiers, no return type, no body.
    // With a body the same shape is a constructor_declaration. In an
    // interface an annotation is enough: `@Option flag()`.
    _bodiless_untyped_method: $ => seq(
      choice(
        seq(repeat($.annotation), repeat1($._modifier)),
        repeat1($.annotation),
      ),
      field('name', $.identifier),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
    ),

    // §4 / §5.14 — method-level generic type parameters
    // (`<T> T identity(T x) { x }`). Distinct from `type_parameters`
    // (used on class/trait/interface) because here the `<` cannot be
    // `token.immediate` — it appears at the start of the declaration
    // with optional whitespace from preceding modifiers / annotations.
    method_type_parameters: $ => seq(
      '<',
      $.type_parameter,
      repeat(seq(',', $.type_parameter)),
      '>',
    ),

    // §4 — `throws E1, E2, …` after a method signature.
    // A type may be annotated (Groovy 3+): `throws @A IOException`.
    throws_clause: $ => prec.left(seq(
      'throws',
      repeat($.annotation),
      $._type,
      repeat(seq(',', repeat($.annotation), $._type)),
    )),

    formal_parameters: $ => seq(
      choice('(', alias($._call_open, '(')),
      optional(seq(
        $.formal_parameter,
        repeat(seq(',', $.formal_parameter)),
      )),
      ')',
    ),

    // §5.13 — formal parameter with optional annotations (Spring's
    // `@PathVariable`, `@RequestBody`, JPA's `@Param`, … — ubiquitous
    // on framework-facing methods), optional type, optional varargs
    // marker (`Type... name`), and optional default value
    // (`name = expr`).
    // `final` is the one modifier a parameter takes, as in Java.
    // `def` may stand before the type, or for it: `def Object x`, `def x`.
    formal_parameter: $ => seq(
      repeat($.annotation),
      // annotations may follow `final`: `final @DelegatesTo(X) Closure c`
      optional(seq('final', repeat($.annotation))),
      optional('def'),
      optional(field('type', choice($._type, $.varargs_type))),
      field('name', $.identifier),
      optional(seq('=', field('default', $._expression))),
    ),

    varargs_type: $ => choice(seq($._type, '...'), '...'),

    // §4 — enum. Closes murtaza64 #36 first half. Constants list
    // (optionally trailing-comma) optionally followed by further
    // class members. Unlike Java, the `;` separating the constant
    // list from the member section is optional in Groovy — mirrors
    // class_body's `repeat(choice($._class_member, ';'))` rather than
    // gating the whole member section behind a required leading `;`.
    // Constants may carry constructor arguments (`RED(0xff0000)`);
    // v1 supports that via the optional argument_list.
    enum_declaration: $ => seq(
      repeat($.annotation),
      repeat($._modifier),
      'enum',
      field('name', $.identifier),
      optional($.super_interfaces),
      field('body', $.enum_body),
    ),

    enum_body: $ => seq(
      '{',
      optional($._line_break),
      optional(seq(
        $._enum_item,
        repeat(seq(',', optional($._line_break), $._enum_item)),
        optional(','),
      )),
      repeat(choice($._class_member, ';', $._line_break)),
      '}',
    ),

    // A constant may have a body: `ONE { String f() { 'a' } }`. Not after
    // arguments, where `E() { … }` is a constructor.
    enum_constant: $ => seq(
      repeat($.annotation),
      field('name', $.identifier),
      optional(choice(
        field('arguments', $.argument_list),
        field('body', $.class_body),
      )),
    ),

    // `THREE() { … },`: arguments and a body, which a following `,` tells
    // apart from a constructor. prec.dynamic: where both fit, the
    // constructor wins as before.
    _enum_constant_with_body: $ => prec.dynamic(-1, seq(
      repeat($.annotation),
      field('name', $.identifier),
      field('arguments', $.argument_list),
      field('body', $.class_body),
    )),

    _enum_item: $ => choice(
      $.enum_constant,
      alias($._enum_constant_with_body, $.enum_constant),
    ),

    // §4 — record (Groovy 4+). `record Name(Type a, Type b) [{ body }]`.
    // `record` is a contextual keyword — tree-sitter's word: directive
    // keeps `record` usable as a plain identifier elsewhere.
    // Access modifiers and `static` only: `final record = x` declares a
    // variable named `record`.
    // prec.dynamic: with modifiers, `private static record R(…) { … }` can
    // also read as a method returning `record`; the record wins.
    record_declaration: $ => prec.right(prec.dynamic(1, seq(
      repeat($.annotation),
      repeat(choice('public', 'protected', 'private', 'static')),
      alias($._record_keyword, 'record'),
      field('name', $.identifier),
      // Groovy 4: `record Pair<A, B>(A a, B b)`
      optional(field('type_parameters', $.type_parameters)),
      field('components', $.record_components),
      optional($.super_interfaces),
      optional(field('body', alias($._record_body, $.class_body))),
    ))),

    // A record's body may also hold a compact constructor (Groovy 4):
    // `record Fruit(String name) { Fruit { assert name } }`.
    _record_body: $ => seq(
      '{',
      repeat(choice(
        $._class_member,
        alias($._compact_constructor, $.constructor_declaration),
        $.instance_initializer,
        ';',
        $._line_break,
      )),
      '}',
    ),

    // After modifiers the `{` must be on the name's line: Groovy reads
    // `public R` then `{` on the next line as a field and an initializer.
    _compact_constructor: $ => prec(-1, seq(
      ...memberModifiers($),
      field('name', $.identifier),
      optional($._command_closure_start),
      field('body', $.block),
    )),

    record_components: $ => seq(
      choice('(', alias($._call_open, '(')),
      optional(seq(
        $.record_component,
        repeat(seq(',', $.record_component)),
      )),
      ')',
    ),

    record_component: $ => seq(
      repeat($.annotation),
      field('type', $._type),
      field('name', $.identifier),
      // Groovy 4 component default: `record R(boolean b = false)`
      optional(seq('=', field('default', $._expression))),
    ),

    // §4 — package and import. Trailing `;` is optional in Groovy;
    // we accept both forms by not requiring it.
    package_declaration: $ => seq(
      repeat($.annotation),
      'package',
      field('name', $.qualified_name),
    ),

    // §4 — `import [static] qualified.Name [.*] [as Alias]`.
    // `.*` is a single anonymous token so it lexes distinctly from
    // a continuation of qualified_name (`.identifier`). Wildcard
    // imports only appear in this position, so the lexer-wide
    // longest-match is safe.
    import_declaration: $ => seq(
      repeat($.annotation),
      'import',
      optional('static'),
      field('name', $.qualified_name),
      optional(token('.*')),
      optional(seq('as', field('alias', $.identifier))),
    ),

    qualified_name: $ => seq(
      $.identifier,
      repeat(seq('.', $.identifier)),
    ),

    // §4 / §5.12 — `def x = …`, `var x = …`, and `Type x = …`.
    // The typed form requires every declarator to carry an
    // `= value` initializer; bare `Type x` without initializer
    // would conflict with method_invocation `Type(x)` and is
    // disallowed in v1. Multiple declarators per statement
    // (`def a = 1, b = 2`) are supported in all three forms.
    // `final` is the one modifier Groovy (like Java) allows on a local.
    // `final x = 1` with no type or `def` is the modifier-only form;
    // the variable is dynamically typed, as with `def`.
    //
    // A declaration led by annotations is another form: `@Field String
    // NAME = 'x'`, `@Field final Map m = [:]`, `@Field def x`, the script
    // field idiom of every Jenkins shared library. The annotation already
    // marks it as a declaration, so modifiers, an uninitialized declarator
    // and an implicit type (`@Field NAME = 1`) are all allowed.
    local_variable_declaration: $ => prec.right(choice(
      seq(
        optional('final'),
        // `def final x = 1` puts the modifier after `def`.
        choice('def', seq('def', 'final'), alias($._var_keyword, 'var'), seq('def', field('type', $._type))),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      // `def @Deprecated x = 1`: annotations after `def`.
      seq(
        'def',
        repeat1($.annotation),
        optional(field('type', $._type)),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      // A generic or array type can't start a command call, so a declarator
      // after one needs no value: `List<String> names`, `byte[] body`.
      prec.dynamic(1, seq(
        optional('final'),
        field('type', choice($.generic_type, $.array_type)),
        alias($._bare_declarator, $.variable_declarator),
        // a later one may take a value: `List<String> a, b = null`
        repeat(seq(',', $.variable_declarator)),
      )),
      // prec.dynamic: `String t = z` is one declaration, not `String` and
      // `t = z` (statements need no separator in this grammar).
      // A later declarator may go without a value: `Object a = f(), b`.
      prec.dynamic(1, seq(
        optional('final'),
        field('type', $._type),
        $._initialized_declarator,
        repeat(seq(',', choice($._initialized_declarator, $._bare_declarator))),
      )),
      // Modifiers on a script variable, as on a field:
      // `public static final long IM = 139968`.
      // `final` may lead: `final static long N = 1`.
      prec.dynamic(1, seq(
        optional('final'),
        choice('public', 'protected', 'private', 'static'),
        repeat($._modifier),
        field('type', $._type),
        $._initialized_declarator,
        repeat(seq(',', $._initialized_declarator)),
      )),
      // The same with no type, a dynamically typed local in Groovy 2.4, 3 and
      // 4: `private static final X = 1`, `static final Y = 2`, `private z = 3`.
      // The token after the first name decides: `=` here, a name for the
      // typed form, `(` for a script method.
      seq(
        optional('final'),
        choice('public', 'protected', 'private', 'static'),
        repeat($._modifier),
        $._initialized_declarator,
        repeat(seq(',', $._initialized_declarator)),
      ),
      // §5.10 — a type on the line before the name is a separate path that
      // loses to any other reading: `a.b` and then `bar = 1` on the next
      // line are two statements, though the parser split at the `.` before
      // it saw the line break.
      prec.dynamic(-3, seq(
        optional('final'),
        field('type', $._type),
        $._line_break,
        $._initialized_declarator,
        repeat(seq(',', $._initialized_declarator)),
      )),
      seq(
        'final',
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      // After `final` a type and a name can't be a command call, so no
      // declarator needs a value: `final String a, b`, `final int c`.
      // prec.dynamic: not `final String` and then the statement `a`.
      prec.dynamic(1, seq(
        'final',
        field('type', $._type),
        alias($._bare_declarator, $.variable_declarator),
        repeat(seq(',', $.variable_declarator)),
      )),
      // `@A final x = 1` with `final` as the token, not a `_modifier`: as the
      // first statement of a block the parser would otherwise drop the block
      // for a closure with the parameter `@A final x`; this way both stay
      // open until `->` or the `=` decides. The untyped form loses to the
      // typed one as below.
      seq(
        repeat1($.annotation),
        'final',
        field('type', $._type),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      prec.dynamic(-1, seq(
        repeat1($.annotation),
        'final',
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      )),
      // Annotations may also follow modifiers: `static @Field x = 1`,
      // `final @Deprecated x = 1`.
      seq(
        annotatedModifiers($),
        choice('def', alias($._var_keyword, 'var'), field('type', $._type)),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      // prec.dynamic: with no type, the name could be the type of a typed
      // declaration or the return type of a method, which win:
      // `@Field static final int N = 1` is not `int` and then `N = 1`, and
      // `@Deprecated private boolean f() { }` is not `boolean` and a call.
      prec.dynamic(-1, seq(
        annotatedModifiers($),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      )),
    )),

    // Groovy 3 and 4 take `in` and `as` as names: `InputStream in = null`,
    // `AssertStatement as = s`.
    _initialized_declarator: $ => seq(
      field('name', choice($.identifier, alias('in', $.identifier), alias('as', $.identifier))),
      '=',
      field('value', $._value),
    ),

    // `in` too, after a type: `final InputStream in` (Groovy 3 and 4).
    _bare_declarator: $ => seq(field('name', choice($.identifier, alias('in', $.identifier)))),

    variable_declarator: $ => seq(
      field('name', $.identifier),
      optional(seq('=', field('value', $._value))),
    ),

    // The value of a declaration or assignment, which may be a command call:
    // `def r = sh returnStdout: true, script: 'x'`.
    _value: $ => choice(
      $._expression,
      alias($._value_command, $.command_chain),
      alias($._lambda_command, $.command_chain),
    ),

    // A spaced call whose one argument is a lambda with parenthesized
    // parameters (Groovy 3+): `configurations.all (Configuration it) -> { }`,
    // `def r = foo () -> { }`. See `_command_lambda_open`.
    _lambda_command: $ => seq(
      field('receiver', choice(
        $.identifier,
        $.field_access,
        $.safe_navigation_expression,
        $.spread_dot_expression,
      )),
      field('argument', alias($._command_lambda, $.lambda_expression)),
    ),

    _command_lambda: $ => prec.right(PREC.ASSIGN, seq(
      field('parameters', alias($._command_lambda_parameters, $.formal_parameters)),
      '->',
      field('body', choice($.block, $._value)),
    )),

    _command_lambda_parameters: $ => seq(
      alias($._command_lambda_open, '('),
      optional(seq(
        $.formal_parameter,
        repeat(seq(',', $.formal_parameter)),
      )),
      ')',
    ),

    // §5.5 — a command call as a value. Its first argument starts on the
    // receiver's line (`_command_value_start`), and may be any expression,
    // a name included, since a value can't be a declaration: `x = foo bar`.
    // prec.right: a comma after an argument starts the next argument.
    _value_command: $ => prec.right(seq(
      // also `def saved = [a, b]*.save flush: true`
      field('receiver', choice(
        $.identifier,
        $.field_access,
        $.safe_navigation_expression,
        $.spread_dot_expression,
      )),
      $._command_value_start,
      field('argument', choice(
        $._expression,
        alias($._command_named_argument, $.named_argument),
        alias($._command_quoted_named_argument, $.named_argument),
      )),
      repeat($._command_later_argument),
    )),

    // §4 — `def (a, b) = expr`. Closes murtaza64 #22. v1 requires
    // the leading `def`; the no-`def` form `(a, b) = expr` lands
    // later (it shares a prefix with parenthesized_expression and
    // needs a conflict declaration).
    //
    // Each name may carry a type: `def (String foo, int bar) = ["foo", 42]`.
    // also annotated: `@Deprecated def (int q, int r) = [1, 2]`
    multi_assignment_declaration: $ => seq(
      repeat($.annotation),
      // `final (a, b) = [1, 2]`: `final` alone stands in for `def`.
      choice(seq('final', optional('def')), 'def'),
      '(',
      $._multi_assignment_target,
      repeat(seq(',', $._multi_assignment_target)),
      ')',
      '=',
      field('value', $._expression),
    ),

    // The last name may take the rest: `def (head, *tail) = list`.
    _multi_assignment_target: $ => seq(
      optional(field('type', $._type)),
      optional('*'),
      $.variable_declarator,
    ),

    // §3.2 — `(a, b) = [1, 2]` assigns existing variables; as an
    // expression it may sit inside parentheses (`((a, b) = l) == l`).
    multi_assignment_expression: $ => prec.right(PREC.ASSIGN, seq(
      $._multi_assignment_names,
      '=',
      field('right', $._expression),
    )),

    // No precedence here, so `(a, b)` stays open to a lambda's parameter
    // list until `=` or `->` decides.
    _multi_assignment_names: $ => seq(
      '(',
      field('left', $.identifier),
      repeat1(seq(',', field('left', $.identifier))),
      ')',
    ),

    // §4 / §10 row #37 — Jenkins-style `pipeline { … }` block.
    // Closes murtaza64 #37 (pipeline as a regular statement, not
    // a file-trailer-only construct). The body is a closure, since
    // that's how Jenkins's DSL is shaped — `pipeline` is followed
    // by a closure literal with nested stages, steps, etc.
    pipeline_statement: $ => seq(
      alias($._pipeline_keyword, 'pipeline'),
      field('body', $.closure),
    ),

    // §5.9 — annotation usage. Lands on its own here; class /
    // method / record / enum declarations gain an optional
    // leading `repeat($.annotation)` so a `@Foo @Bar class X { }`
    // attaches the annotations to the class node.
    //
    // Annotation arguments are an argument list whose values may also be
    // annotations or lists of them, as Spring and Spock metadata use:
    // `@ContextHierarchy(@ContextConfiguration(locations = "x"))`,
    // `@FeatureMetadata(blocks = [@BlockMetadata(kind = SETUP)])`.
    annotation: $ => seq(
      '@',
      field('name', $.qualified_name),
      optional(field('arguments', alias($._annotation_arguments, $.argument_list))),
    ),

    _annotation_arguments: $ => seq(
      choice('(', alias($._call_open, '(')),
      optional(seq(
        $._annotation_argument,
        repeat(seq(',', $._annotation_argument)),
        optional(','),
      )),
      ')',
    ),

    // A generic type is a value here, `@Scaffold(Foo<User>)`; outside
    // annotations `f(a<b)` keeps `<` an operator.
    // A Java array of values in braces is one too: `@Target({TYPE, METHOD})`.
    _annotation_argument: $ => choice(
      $._argument,
      alias($._brace_array, $.array_initializer),
      $.generic_type,
      $.annotation,
      alias($._annotation_list, $.list_literal),
      alias($._annotation_element_pair, $.assignment_expression),
    ),

    _annotation_element_pair: $ => seq(
      field('left', $.identifier),
      field('operator', '='),
      field('right', choice($.annotation, alias($._annotation_list, $.list_literal))),
    ),

    _annotation_list: $ => seq(
      '[',
      optional(seq(
        $._annotation_list_element,
        repeat(seq(',', $._annotation_list_element)),
        optional(','),
      )),
      ']',
    ),

    _annotation_list_element: $ => choice(
      $._expression,
      $.annotation,
    ),

    // §4 — `for (x in xs)`, plus the Java-compatible enhanced-for
    // form Groovy accepts side by side: `for (Type x : xs)` / `for
    // (def x : xs)`. The `in` / `:` keyword here is a structural
    // delimiter, not the membership_expression operator or a
    // labelled_statement colon — context disambiguates because the
    // for-header is not an expression position.
    //
    // Groovy 5 adds `for await (x in xs)` and an index variable before the
    // loop variable: `for (int i, v in list)`.
    for_in_statement: $ => seq(
      'for',
      optional('await'),
      '(',
      optional(seq(
        optional(field('index_type', $._type)),
        field('index', $.identifier),
        ',',
      )),
      // also annotated: `for (@Deprecated String i in l)`
      repeat($.annotation),
      optional('final'),
      optional(choice('def', alias($._var_keyword, 'var'), field('type', $._type))),
      // `as` is a name too (Groovy 3 and 4): `for (AssertStatement as : l)`
      field('variable', choice($.identifier, alias('as', $.identifier))),
      choice('in', ':'),
      field('value', $._expression),
      ')',
      field('body', choice($.block, $._statement)),
    ),

    expression_statement: $ => choice(
      $._expression,
      alias($._static_assignment, $.assignment_expression),
    ),

    // §3.2 PRIMARY — minimal alternative set for now. Every primary
    // expression form from §3.2 / §5 will land in this choice over
    // subsequent iterations.
    _expression: $ => choice(
      $.identifier,
      $.number_literal,
      $.string_literal,
      $.boolean_literal,
      $.null_literal,
      $.parenthesized_expression,
      $.unary_expression,
      $.list_literal,
      $.map_literal,
      $.closure,
      $.lambda_expression,
      $.binary_expression,
      $.range_expression,
      $.identity_expression,
      $.spaceship_expression,
      $.regex_find_expression,
      $.regex_match_expression,
      $.ternary_expression,
      $.elvis_expression,
      $.logical_implication_expression,
      $.assignment_expression,
      $.multi_assignment_expression,
      $.power_expression,
      $.subscript_expression,
      alias($._empty_list_subscript, $.subscript_expression),
      $.safe_subscript_expression,
      $.method_invocation,
      $.update_expression,
      $.unary_update_expression,
      $.field_access,
      $.safe_navigation_expression,
      $.safe_chain_dot_expression,
      $.spread_dot_expression,
      $.method_pointer_expression,
      $.direct_field_access_expression,
      $.method_reference_expression,
      $.object_creation_expression,
      $.array_creation_expression,
      $.parenthesized_type_cast,
      $.cast_expression,
      $.membership_expression,
      $.instanceof_expression,
      $.switch_expression,
      // §4 — an array type is a value too: `byte[].class`,
      // `getBean(String[], name)`, `String[]::new`, `List[]` in a data table.
      alias($._array_type_value, $.array_type),
      alias($._generic_type_value, $.generic_type),
    ),

    // §5.2 — `static.unused = { }` in a metaclass DSL. A statement only, where
    // `static` is a keyword anyway; in an expression it stays one, so
    // `[static: true]` keeps its key.
    _static_assignment: $ => prec.right(PREC.ASSIGN, seq(
      field('left', alias(seq(
        field('object', alias('static', $.identifier)),
        '.',
        field('field', $._property_name),
      ), $.field_access)),
      field('operator', '='),
      field('right', $._value),
    )),

    // Not on a generic type: `<` right after a name would then lex as a type
    // argument opener in every expression, breaking `i<n`.
    _array_type_value: $ => seq(
      choice(
        alias($.identifier, $.type_identifier),
        $.qualified_type,
        alias($._array_type_value, $.array_type),
      ),
      alias($._subscript_open, '['),
      ']',
    ),

    // §4 — Groovy 4 switch-expression. Single rule for both classic
    // `case … :` and arrow `case … -> …` branches; arrow form
    // closes murtaza64 #36 (second half). Used as a statement via
    // expression_statement, or as an expression directly per the
    // §8.2 #36 regression.
    switch_expression: $ => seq(
      'switch',
      '(',
      field('value', $._expression),
      ')',
      field('body', $.switch_block),
    ),

    switch_block: $ => seq(
      '{',
      repeat(choice(
        $.switch_case,
        $.switch_arrow_case,
        $.switch_default,
      )),
      '}',
    ),

    // Groovy 4 switch expressions take several labels before a colon
    // too: `case 6, 9: yield 'a'`.
    switch_case: $ => seq(
      'case',
      field('value', $._case_value),
      repeat(seq(',', field('value', $._case_value))),
      choice(':', alias($._expression_colon, ':')),
      repeat(choice($._statement, ';', $._line_break)),
    ),

    // The arrow body may be terminated by `;` (`case 1 -> foo();`),
    // matching the Java/Groovy `case L -> expr;` arrow form. The `;`
    // is anonymous and absorbed here rather than by the switch block,
    // which has no statement-list of its own.
    // Several labels share an arrow: `case 1, 2 -> …`.
    // A Groovy 5 type pattern binds a name, with an optional guard:
    // `case Integer i when i > 0 -> …`.
    switch_arrow_case: $ => seq(
      'case',
      field('value', choice($._case_value, $.type_pattern)),
      repeat(seq(',', field('value', $._case_value))),
      alias($._case_arrow, '->'),
      field('body', choice($.block, $._statement)),
      optional(';'),
    ),

    // A name before `->` is the case's label, not a lambda's parameter
    // (`case FOO -> 1`, `case a < b -> 3`): the scanner's `_case_arrow`.
    _case_value: $ => $._expression,

    type_pattern: $ => seq(
      field('type', $._type),
      field('name', $.identifier),
      optional(seq('when', field('guard', $._expression))),
    ),

    switch_default: $ => seq(
      'default',
      choice(
        seq(':', repeat(choice($._statement, ';', $._line_break))),
        seq('->', field('body', choice($.block, $._statement)), optional(';')),
      ),
    ),

    // §3.2 — `x as Type`. RELATIONAL level (10), left-associative.
    // RHS is `_cast_type`, a `_type`-shaped sibling that swaps in
    // `_dotted_type` for `qualified_type` so the trailing
    // `.identifier` of `x as java.util.Map` extends the type
    // through every dot segment (instead of being claimed by
    // `field_access` at higher precedence than `cast_expression`).
    cast_expression: $ => prec.left(PREC.RELATIONAL, seq(
      field('value', $._expression),
      'as',
      field('type', $._cast_type),
    )),

    // §3.2.2 — RHS type of `as`, `instanceof`, and `!instanceof`.
    // Same alternatives as `_type` except `qualified_type` is
    // replaced by `_dotted_type`, whose tail shifts each dot at a
    // precedence tighter than `field_access`.
    // prec.dynamic: `x as A.B<C>` is a cast to a generic type, never the
    // comparison chain `(x as A.B) < C > …` that `_dotted_type` would win.
    // A dotted name may also take type arguments or `[]` here:
    // `x as java.util.concurrent.Callable<T>`, `[] as A.B[]`, `[] as A.B<C>[]`.
    _cast_type: $ => choice(
      alias($.identifier, $.type_identifier),
      $.array_type,
      alias($._dotted_type, $.qualified_type),
      prec.dynamic(2, $.generic_type),
      prec.dynamic(2, alias($._dotted_generic_type, $.generic_type)),
      alias($._dotted_array_type, $.array_type),
    ),

    _dotted_generic_type: $ => seq(
      field('base', alias($._dotted_type, $.qualified_type)),
      field('arguments', choice(
        $.type_arguments,
        alias($._value_type_arguments, $.type_arguments),
      )),
    ),

    _dotted_array_type: $ => prec(PREC.ARRAY_TYPE, seq(
      choice(
        alias($._dotted_type, $.qualified_type),
        alias($._dotted_generic_type, $.generic_type),
        alias($._dotted_array_type, $.array_type),
      ),
      alias($._subscript_open, '['),
      ']',
    )),

    // prec.dynamic: with a dotted command receiver in the grammar, the
    // parser can also carry `(x as java).util` to the end; the type wins.
    _dotted_type: $ => prec.dynamic(1, seq(
      alias($.identifier, $.type_identifier),
      $._dotted_type_tail,
    )),

    _dotted_type_tail: $ => prec.right(PREC.ACCESS + 1, seq(
      '.',
      alias($.identifier, $.type_identifier),
      optional($._dotted_type_tail),
    )),

    // §3.2 — `x in xs` and `x !in xs`. RELATIONAL level. Both
    // operators share one node kind; the `operator` field
    // distinguishes them. `!in` is wrapped in `token(prec(1, …))`
    // so it lexes as a single token despite the embedded space
    // requirement implied by §3.2.2's trailing-context rule —
    // Apache's `NOT_IN` predicate is approximated by requiring at
    // least one trailing whitespace, `[`, `(`, or `{` after `!in`.
    // §3.2 / §3.2.2 — `x in xs` and `x !in xs`. `!in` is a single
    // token whose match requires a trailing whitespace / bracket
    // character (Apache `NOT_IN` predicate) so `a !inXs` does not
    // mis-tokenise as `!in` followed by `Xs`.
    membership_expression: $ => {
      const ops = [
        'in',
        token(prec(1, seq('!in', /[ \t\r\n\[({]/))),
        alias($._not_in_open, '!in'),
      ];
      return choice(...ops.map(op => prec.left(PREC.RELATIONAL, seq(
        field('element', $._expression),
        field('operator', op),
        field('collection', $._expression),
      ))));
    },

    // §3.2 / §3.2.2 — `x instanceof T` and `x !instanceof T`.
    // `!instanceof` is a single token whose match requires a
    // trailing whitespace (Apache `NOT_INSTANCEOF` predicate) so
    // `a !instanceofX` does not mis-tokenise as `!instanceof`
    // followed by identifier `X`.
    instanceof_expression: $ => {
      const ops = [
        'instanceof',
        token(prec(1, seq('!instanceof', /[ \t\r\n]/))),
      ];
      return choice(...ops.map(op => prec.left(PREC.RELATIONAL, seq(
        field('value', $._expression),
        field('operator', op),
        field('type', $._cast_type),
        optional(field('name', $.identifier)),
      ))));
    },

    // §3.2 / §5.15 — `new Foo(args)`. We only handle the plain
    // constructor-call form here; array creation `new Foo[10]`,
    // generic types, and array initialisers land later. PREC.PRIMARY
    // so it composes with the access tier from the left
    // (`new Foo().bar`).
    // A `{` after the arguments on their line starts an anonymous class body:
    // `new Runnable() { void run() { } }` (the zero-width `_command_closure_start`
    // matches only a `{` on the line). A `{` on the next line may start a class
    // body too, inside brackets, `foo(new Runnable()` then `{ void run() { } })`,
    // or a closure argument of the new object, `new Object()` then `{ -> 3 }`, as
    // Groovy 2.4, 3 and 4 read it. Both stay alive until one fails; where both
    // parse, the class body wins. Its dynamic precedence outweighs the +1 that
    // each declaration in the closure's body adds: `{ int x = 1 }` in a call's
    // parentheses is a class body to Groovy. Where a statement may end, a `{` on
    // the next line is always a closure (`_new_closure_start`, see
    // method_invocation): `def a = new Object()` then `{ }`.
    object_creation_expression: $ => {
      const head = [
        'new',
        repeat($.annotation),
        field('type', $._type),
        field('arguments', alias($._new_argument_list, $.argument_list)),
      ];
      return choice(
        prec(PREC.PRIMARY, seq(
          ...head,
          optional(seq($._command_closure_start, field('body', $.class_body))),
        )),
        prec(PREC.PRIMARY, prec.dynamic(100, seq(...head, field('body', $.class_body)))),
      );
    },

    // §3.2 / §5.15 — array creation: `new byte[10]`, `new int[2][3]`,
    // `new String[3][]`, and with a Java array initializer
    // `new String[] { 'a', 'b' }`.
    array_creation_expression: $ => prec.right(PREC.PRIMARY, seq(
      'new',
      field('type', choice(
        alias($.identifier, $.type_identifier),
        $.qualified_type,
        $.generic_type,
      )),
      choice(
        seq(repeat1($.dimension_expression), optional($._trailing_dimensions)),
        seq(repeat1($.dimension), field('value', $.array_initializer)),
      ),
    )),

    // `[` on the type's line lexes as `_subscript_open` (§5.10).
    // A dimension may be annotated (Groovy 3+): `new String @A [1]`.
    dimension_expression: $ => seq(
      optional($._dimension_annotations),
      alias($._subscript_open, '['),
      $._expression,
      ']',
    ),

    dimension: $ => seq(optional($._dimension_annotations), alias($._subscript_open, '['), ']'),

    // Right-recursive so each `[]` after the first stays a dimension:
    // `new int[10][][]`.
    _trailing_dimensions: $ => prec.right(PREC.PRIMARY, seq($.dimension, optional($._trailing_dimensions))),

    // Elements are expressions, or nested initializers with a comma
    // (`{{1, 2}, {3, 4}}`); a nested `{x}` is a closure.
    array_initializer: $ => seq(
      '{',
      optional(seq(
        $._array_initializer_element,
        repeat(seq(',', $._array_initializer_element)),
        optional(','),
      )),
      '}',
    ),

    _array_initializer_element: $ => choice(
      $._expression,
      alias($._brace_array, $.array_initializer),
    ),

    // `{a, b}`: the comma tells it from a closure.
    _brace_array: $ => seq(
      '{',
      $._array_initializer_element,
      repeat1(seq(',', $._array_initializer_element)),
      optional(','),
      '}',
    ),

    // §5.15 — C-style cast `(Foo) x`. Distinct from
    // `parenthesized_expression` followed by something (resolved
    // by tree-sitter's GLR; declared as a conflict if needed).
    // PREC.UNARY right-associative so `(Foo) -x` parses as
    // `(Foo)(-x)` and the type binds to the rightward expression.
    // prec.dynamic: `(T) x.y` followed by `bar = 1` on the next line is a
    // cast and an assignment, not `(T)` and a declaration `x.y bar = 1`; it
    // must outweigh the typed declaration's preference. Only when the value
    // starts on the line of the `)` (`_cast_value_start`, zero width). A
    // value that starts with a name, string or number must be on that line:
    // after `x = (e.user)` a statement on the next line is not a cast's
    // value. The plain form takes the other values (`(int) -1`, `(T) [a]`).
    // The type may be annotated (Groovy 3+): `(@A String) e`.
    parenthesized_type_cast: $ => prec.right(PREC.UNARY, choice(
      prec.dynamic(2, seq(
        '(',
        repeat($.annotation),
        field('type', $._type),
        ')',
        $._cast_value_start,
        field('value', $._expression),
      )),
      seq(
        '(',
        repeat($.annotation),
        field('type', $._type),
        ')',
        field('value', choice(
          $.unary_expression,
          $.unary_update_expression,
          $.parenthesized_expression,
          $.parenthesized_type_cast,
          $.list_literal,
          $.map_literal,
          $.closure,
          $.lambda_expression,
        )),
      ),
      // A `(` value on the next line is a cast only on a path that loses
      // to any other reading: `v = (b)` and then `(a + b).c()` are two
      // statements.
      prec.dynamic(-3, seq(
        '(',
        repeat($.annotation),
        field('type', $._type),
        ')',
        $._line_break,
        field('value', choice(
          $.parenthesized_expression,
          $.parenthesized_type_cast,
          $.lambda_expression,
        )),
      )),
    )),

    // §4 — `_type`. Supports unqualified type identifiers,
    // qualified types (`java.util.List`), array types
    // (`int[]`, `String[][]`), and generic types (`List<String>`,
    // `Map<String, Integer>`). `generic_type` always carries explicit
    // angle brackets, so it never competes with comparison operators
    // in expression context (which only reach `_type` through a
    // separate parse stack).
    _type: $ => choice(
      alias($.identifier, $.type_identifier),
      $.array_type,
      $.qualified_type,
      $.generic_type,
    ),

    // Near-twin of `_dotted_type` below; the two cannot be merged
    // without breaking either field-access (which needs
    // `qualified_type` to NOT outrank `field_access`) or
    // cast / instanceof RHS (which needs `_dotted_type` to outrank
    // `field_access`). Edits here should be mirrored in
    // `_dotted_type` / `_dotted_type_tail`.
    qualified_type: $ => prec.left(seq(
      alias($.identifier, $.type_identifier),
      repeat1(seq('.', alias($.identifier, $.type_identifier))),
    )),

    // A dimension may be annotated (Groovy 3+): `String @A [] s`.
    array_type: $ => prec(PREC.ARRAY_TYPE, seq(
      $._type,
      optional($._dimension_annotations),
      alias($._subscript_open, '['),
      ']',
    )),

    // Only when `[` follows the annotations on their line: `new int[0]` then
    // `@Override` on the next line starts a member, not another dimension.
    _dimension_annotations: $ => seq($._dimension_annotation_start, repeat1($.annotation)),

    // §4 / §5.14 — Java-style generic type arguments. Allows nested
    // generics (`Map<String, List<Integer>>`) and bounded wildcards
    // (`? extends Foo`, `? super Bar`). The `<` is `token.immediate`
    // so it only matches when the angle bracket directly follows the
    // base name with no intervening whitespace — `a < b` still parses
    // as a binary expression because of the space before `<`.
    generic_type: $ => prec.right(PREC.ARRAY_TYPE, seq(
      field('base', choice(
        alias($.identifier, $.type_identifier),
        $.qualified_type,
      )),
      field('arguments', choice(
        $.type_arguments,
        alias($._value_type_arguments, $.type_arguments),
      )),
    )),

    // A generic type as a value: `Mock(Supplier<Long>)`, `typeOf(List<X>)`,
    // `HashMap<String, Object>.class`, `Mono<String>.fromCallable { }`.
    _generic_type_value: $ => prec.right(PREC.ARRAY_TYPE, seq(
      field('base', choice(
        alias($.identifier, $.type_identifier),
        $.qualified_type,
      )),
      field('arguments', alias($._value_type_arguments, $.type_arguments)),
    )),

    _spaced_type_arguments: $ => seq(
      '<',
      $._type_argument,
      repeat(seq(',', $._type_argument)),
      '>',
    ),

    _value_type_arguments: $ => seq(
      alias($._type_arguments_open, '<'),
      optional(seq(
        $._type_argument,
        repeat(seq(',', $._type_argument)),
      )),
      '>',
    ),

    // Empty type-argument list is the diamond operator (`new
    // ArrayList<>()`) — ubiquitous since Java 7 / Groovy's matching
    // support, so `<>` has to parse as zero type arguments rather
    // than requiring at least one.
    type_arguments: $ => seq(
      token.immediate('<'),
      optional(seq(
        $._type_argument,
        repeat(seq(',', $._type_argument)),
      )),
      '>',
    ),

    // A type argument may be annotated, `List<@A ? extends Object>`, and so may
    // an array dimension (see `array_type`): `Consumer<@T1 String @T2 []>`.
    _type_argument: $ => seq(
      repeat($.annotation),
      choice($._type, $.wildcard),
    ),

    // The bound may be annotated: `? extends @A Object`.
    wildcard: $ => seq(
      '?',
      optional(seq(
        choice('extends', 'super'),
        repeat($.annotation),
        $._type,
      )),
    ),

    // §3.1 ACCESS tier / §3.2 — seven distinct node kinds for the
    // dot family. All at PREC.ACCESS left-associative so chains like
    // `a.b.c` parse `(a.b).c`. Right operand is the property /
    // field / method name (an identifier; method_reference also
    // accepts the `new` keyword per §5.14). Each operator emits a
    // unique node kind so downstream tooling (`get_op_type()`) can
    // discriminate without tree-shape heuristics.

    // `Collections.<String>emptyList()` passes explicit type arguments.
    // A space may come before them: `Arrays. < T > asList(a)`.
    field_access: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.',
      optional(field('type_arguments', choice(
        $.type_arguments,
        alias($._spaced_type_arguments, $.type_arguments),
      ))),
      field('field', $._property_name),
    )),

    safe_navigation_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '?.',
      optional(field('type_arguments', $.type_arguments)),
      field('property', $._property_name),
    )),

    // Groovy 4+ — see SPECIFICATION.md §3.2. Unlike `?.`, which only
    // short-circuits when the immediate receiver is null, `??.`
    // propagates null through the whole chain.
    safe_chain_dot_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '??.',
      field('property', $._property_name),
    )),

    // Both may pass explicit type arguments: `l*.<String>m()`, `l?.<String>m()`.
    spread_dot_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '*.',
      optional(field('type_arguments', $.type_arguments)),
      field('property', $._property_name),
    )),

    method_pointer_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.&',
      field('method', $._property_name),
    )),

    // `?.@` and `*.@` are the null-safe and spread forms.
    direct_field_access_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      // Groovy also takes a line break or spaces before the `@`:
      // `a*.` then `@child` on the next line.
      choice('.@', '?.@', '*.@', seq(choice('.', '?.', '*.'), '@')),
      field('field', $._property_name),
    )),

    // §5.2 — property / method names after a dot operator (or as a
    // method declaration name) can be either an identifier or a
    // *quoted identifier*: a string literal that escapes spaces,
    // keywords, or special chars. `map."key with spaces"` and
    // `def "abstract"() { … }` both rely on this.
    // A declared method's name: reserved words excluded (a statement keyword
    // on the next line is never a method name), or a quoted name.
    _method_name: $ => choice(
      $.identifier,
      alias($.string_literal, $.quoted_identifier),
    ),

    _property_name: $ => choice(
      reserved('properties', $.identifier),
      alias($.string_literal, $.quoted_identifier),
      $.parenthesized_expression,
    ),

    // §5.14 — method reference. Groovy 3+. The RHS is either an
    // identifier (`String::length`) or the `new` keyword
    // (`Foo::new`), also `class` (Groovy 3 and 4): `foo(x, A::class)`.
    method_reference_expression: $ => prec.left(PREC.ACCESS, seq(
      field('target', $._expression),
      '::',
      field('name', choice($.identifier, alias($.string_literal, $.quoted_identifier), 'new', 'class')),
    )),

    // §3.2 — postfix `++` / `--`. Distinct node kind from the
    // prefix form (`unary_update_expression`) per the spec table.
    // PREC.POSTFIX, left-associative.
    update_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('operand', $._expression),
      field('operator', choice('++', '--')),
    )),

    // §3.2 — prefix `++` / `--`. Apache groups these with UNARY_ADD
    // (level 3); we use PREC.UNARY to match.
    unary_update_expression: $ => prec.right(PREC.UNARY, seq(
      field('operator', choice('++', '--')),
      field('operand', $._expression),
    )),

    // §3.2 POSTFIX / §3.2.1 — `f(args)` and the subscript-yielding-
    // callable composition `arr[i](args)`. The callee slot is any
    // expression so subscript / future field_access / future
    // safe_navigation chains compose naturally. PREC.POSTFIX
    // matches subscript so chains like `obj[i](x)[j]` work.
    //
    // Trailing closure (§5.5's command_chain covers the bare-
    // identifier statement form; this covers the general expression
    // form used constantly on property-access chains: `list.find {
    // it > 5 }`, `list.inject(0) { acc, x -> acc + x }`). Groovy
    // treats a closure literal as the call's last argument whether
    // or not parens are present, so `argument_list` becomes optional
    // once a trailing closure is there to anchor the call.
    //
    // The args-and-closure alternative sits one precedence level
    // above plain POSTFIX so `f(x) { … }` reduces to a single
    // method_invocation node (arguments AND closure fields on the
    // same node) instead of GLR instead reaching the same span by
    // nesting a closure-only method_invocation around a plain call.
    method_invocation: $ => choice(
      prec.left(PREC.POSTFIX + 1, seq(
        field('function', $._expression),
        field('arguments', $.argument_list),
        field('closure', $.closure),
      )),
      prec.left(PREC.POSTFIX, seq(
        field('function', $._expression),
        field('arguments', $.argument_list),
      )),
      prec.left(PREC.POSTFIX, seq(
        field('function', $._expression),
        field('closure', $.closure),
      )),
      // `new X()` then a closure: on the next line where a statement may end,
      // `def a = new Object()` then `{ }`; or on its line when it can't be a
      // class body, `new Object() { -> 3 }`. Groovy calls the new object with it.
      prec.left(PREC.POSTFIX, seq(
        field('function', $.object_creation_expression),
        $._new_closure_start,
        field('closure', $.closure),
      )),
      prec.left(PREC.POSTFIX, seq(
        field('function', $.object_creation_expression),
        field('closure', alias($._new_closure_on_line, $.closure)),
      )),
    ),

    // The closure reading of `new X() {` on its line loses to the class body
    // where both parse. Its weight sits on the `{`, reduced first, not on the
    // whole call: tree-sitter keeps at most six parse versions and drops the
    // lowest weighted. With the weight only at the end, each declaration in
    // the closure's body (+1) outweighed the class body's members, and a body
    // like `int r = 1`, `String c = (String) 's'`, `def f() { c }` lost its
    // class body reading to the version cap.
    _new_closure_on_line: $ => seq(
      $._new_closure_open,
      ...closureBody($),
    ),

    // -32000, near the 16-bit limit of a dynamic precedence: a class body flips to the
    // closure reading only past about 32,000 members (31,000 stay a class body, 33,000
    // flip). At -1000 it flipped at about 1,000.
    _new_closure_open: $ => prec.dynamic(-32000, seq($._command_closure_start, '{')),

    // §3.2 / §5.11 — argument list. Positional args are `_expression`;
    // named args (Groovy's `k: v` in call position) are `map_entry`
    // aliased to `named_argument` so downstream tools can tell them
    // apart from positional values. Spread (`*expr`) lands later.
    // The `(` must be on the line of the expression before it
    // (`_call_open`): `println 'x'` followed by `(1..n).each { }` on the
    // next line is two statements, not a call on `'x'`.
    argument_list: $ => seq(
      alias($._call_open, '('),
      optional(choice(
        $._argument_list_body,
        alias($._argument_list_command, $.command_chain),
      )),
      ')',
    ),

    // §5.5 — a command call as the first argument, which takes the arguments
    // after it too, as Groovy reads it:
    // `form(new Spec().ui new Attachment(), { … })` passes `form` one value.
    // On a bare name (Groovy 3 and 4): `println(g 1)`, `parseText(readJSON file: f)`.
    // Also on a quoted name (Groovy 3 and 4): `f("foo" 1)` is `f(foo(1))`.
    // A name left after the last argument is a property, a quoted one too (Groovy 3 and 4):
    // `f(a 1 b)` is `f(a(1).b)`, `f("a" 1 "b")` is `f(a(1).b)`.
    _argument_list_command: $ => prec.right(seq(
      field('receiver', choice($.identifier, $.field_access, $.string_literal)),
      choice(
        seq($._command_literal_start, field('argument', $._expression)),
        field('argument', alias($._command_named_argument, $.named_argument)),
        // `shadow(implementation project(":lib"))` is `shadow(implementation(project(":lib")))`
        field('argument', alias($._name_call, $.method_invocation)),
      ),
      repeat($._command_later_argument),
      optional(choice($.identifier, $.string_literal)),
    )),

    // After `new Type` the parenthesis may follow a newline: `new File`
    // then `("Foo")` on the next line is one expression.
    _new_argument_list: $ => seq(
      choice('(', alias($._call_open, '(')),
      optional($._argument_list_body),
      ')',
    ),

    // Groovy 2.4 drops an empty argument, `foo(a,,b)` (3 and 4 reject it). The
    // extra commas stay tokens of the list, as they do in list and map literals.
    _argument_list_body: $ => seq(
      $._argument,
      repeat(seq(',', repeat(','), $._argument)),
      optional(','),
    ),

    // A spread map is an argument too: `f(*: m)`. So is a command call with
    // one name argument, as Geb page objects write it: `visibilityOf(el x)`.
    _argument: $ => choice(
      $._expression,
      alias($._argument_command, $.command_chain),
      alias($._call_chain, $.command_chain),
      alias($.map_entry, $.named_argument),
      $.spread_arguments,
      $.spread_map_entry,
    ),

    // The receiver may be dotted (Groovy 3 and 4): `l.add(sc.process f, "x")`.
    // prec.dynamic: in an enum, `E(A b` may also start a constructor's
    // parameters; where both parse, the parameter wins.
    _argument_command: $ => prec.dynamic(-1, seq(
      field('receiver', choice($.identifier, $.field_access)),
      field('argument', $.identifier),
    )),

    // §5.11 — `*expr` only inside argument list / list literal.
    // Distinct from a binary `*` because the LHS is absent.
    spread_arguments: $ => seq(
      '*',
      field('value', $._expression),
    ),

    // §3.1 POSTFIX / §3.2 — subscript and safe-subscript. The
    // distinction `arr?[k]` (safe) vs `arr[k]` (regular) is the
    // explicit ask of dekobon #247 for the `?[]` row. Both at
    // PREC.POSTFIX, left-associative — `a[b][c]` parses as
    // `(a[b])[c]`.
    // `[` must be on the object's line (see `_subscript_open`): a Grails
    // controller action that ends in `[model: value]` on its own line returns
    // a map; it isn't a subscript of the statement before it.
    // Several indexes select several elements: `list[1, 2]`, `map['a', 'b']`.
    // An index may be spread: `(1..5)[*1..3]`.
    // An empty map is an index too: `render [:] as JSON` is `render[:]`.
    // So are named entries, which make one map: `A[x: 1, y: 2]`, `A[*: m]`.
    subscript_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('object', $._expression),
      alias($._subscript_open, '['),
      choice(
        seq(
          field('index', $._subscript_index),
          repeat(seq(',', field('index', $._subscript_index))),
        ),
        ':',
      ),
      ']',
    )),

    _subscript_index: $ => choice(
      $._expression,
      $.spread_arguments,
      alias($.map_entry, $.named_argument),
      $.spread_map_entry,
    ),

    // An empty index is valid on a list literal, `def l = [][]`; elsewhere
    // `x[]` stays an array type. A string takes one too: `"x"[]` is
    // `"x"[[]]`.
    _empty_list_subscript: $ => seq(
      // and a null or a number: `null[]`, `1[]` (they throw at run time)
      field('object', choice($.list_literal, $.string_literal, $.null_literal, $.number_literal)),
      alias($._subscript_open, '['),
      ']',
    ),

    // `?[` is one token. `a ? [b] : c` (with space) still parses
    // as ternary plus list because tree-sitter's lexer never
    // crosses a whitespace boundary to extend a multi-char token.
    // Several or named indexes as for `[`: `a?[1, 2]`, `A?[name: 1]`.
    safe_subscript_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('object', $._expression),
      '?[',
      field('index', $._subscript_index),
      repeat(seq(',', field('index', $._subscript_index))),
      ']',
    )),

    // §3.1 level 2 / §3.2 — `**`. Distinct node kind so metric tools
    // count power separately from multiplicative ops. Right-assoc
    // per Apache: `1 ** 2 ** 3` parses as `1 ** (2 ** 3)`. POWER
    // binds tighter than UNARY (+ -) but looser than UNARY_NOT
    // (! ~) — see PREC table comments.
    power_expression: $ => prec.right(PREC.POWER, seq(
      field('left', $._expression),
      '**',
      field('right', $._expression),
    )),

    // §3.1 level 13.5 / §3.2 — `==>`. Right-associative per Apache
    // (`<assoc=right>`), distinct node kind so downstream tools can
    // discriminate from `=>` (lambda arrow, not yet wired).
    logical_implication_expression: $ => prec.right(PREC.IMPLICATION, seq(
      field('left', $._expression),
      '==>',
      field('right', $._expression),
    )),

    // §3.1 level 15 / §3.2 — single `assignment_expression` node
    // covers every augmented form (`+=`, `-=`, `*=`, `/=`, `%=`,
    // `**=`, `<<=`, `>>=`, `>>>=`, `&=`, `^=`, `|=`) plus plain `=`
    // and Elvis-assign `?=`. The operator field distinguishes them.
    // Right-associative per Apache. ASSIGN is the loosest precedence
    // tier and binds looser than CONDITIONAL above it.
    assignment_expression: $ => {
      const ops = [
        '=',
        '+=', '-=', '*=', '/=', '%=', '**=',
        '<<=', '>>=', '>>>=',
        '&=', '^=', '|=',
        '?=',
        // Groovy 6 null-coalescing assignment.
        '??=',
      ];
      return choice(...ops.map(op => prec.right(PREC.ASSIGN, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._value),
      ))));
    },

    // §3.1 level 14 / §3.2 — ternary and Elvis share the CONDITIONAL
    // precedence level and are both right-associative. Apache groups
    // them in one `conditionalExprAlt` so `a ? b ?: c : d` parses as
    // `a ? (b ?: c) : d` because the middle slot recursively re-enters
    // the same precedence level. We replicate this with two distinct
    // rules at PREC.CONDITIONAL; right-associativity + recursion via
    // `$._expression` give the same result.
    //
    // elvis_expression is the headline ask of
    // dekobon/big-code-analysis #246: downstream tools need to
    // discriminate elvis from ternary by node kind alone.
    ternary_expression: $ => prec.right(PREC.CONDITIONAL, seq(
      field('condition', $._expression),
      choice('?', alias($._ternary_question, '?')),
      field('consequence', $._expression),
      choice(':', alias($._expression_colon, ':')),
      field('alternative', $._expression),
    )),

    elvis_expression: $ => prec.right(PREC.CONDITIONAL, seq(
      field('value', $._expression),
      '?:',
      field('default', $._expression),
    )),

    // §3.2 — distinct-node ops that share precedence with existing
    // binary_expression tiers but emit unique node kinds per the
    // operator-coverage matrix. The integration target (`get_op_type()`
    // in dekobon/big-code-analysis) needs to discriminate these by
    // node kind alone, not by tree-shape heuristics.

    // §3.2 range_expression — inclusive `..`, exclusive `..<` (right),
    // exclusive `<..` (left), exclusive `<..<` (both). SHIFT_OR_RANGE
    // precedence level, left-associative.
    range_expression: $ => {
      const ranges = ['..', '..<', '<..', '<..<'];
      return choice(...ranges.map(op => prec.left(PREC.SHIFT_OR_RANGE, seq(
        field('start', $._expression),
        field('operator', op),
        field('end', $._expression),
      ))));
    },

    // §3.2 identity_expression — `===` (identity) and `!==`
    // (non-identity), EQUALITY level. Per Apache, these live in
    // the equality alternative.
    identity_expression: $ => {
      const ops = ['===', '!=='];
      return choice(...ops.map(op => prec.left(PREC.EQUALITY, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    // §3.2 spaceship_expression — `<=>`, EQUALITY level. Three-way
    // comparison; no `operator` field since there is only one form.
    spaceship_expression: $ => prec.left(PREC.EQUALITY, seq(
      field('left', $._expression),
      '<=>',
      field('right', $._expression),
    )),

    // §3.2 regex_find_expression — `=~`, EQUALITY level. Matches a
    // pattern at any position. Fields are `subject` / `pattern` so
    // downstream tools can tell the regex-bearing operand from the
    // input expression.
    regex_find_expression: $ => prec.left(PREC.EQUALITY, seq(
      field('subject', $._expression),
      '=~',
      field('pattern', $._expression),
    )),

    // §3.2 regex_match_expression — `==~`, EQUALITY level. Anchored
    // whole-string match. Same field shape as regex_find_expression.
    regex_match_expression: $ => prec.left(PREC.EQUALITY, seq(
      field('subject', $._expression),
      '==~',
      field('pattern', $._expression),
    )),

    // §3.2 — single binary_expression node with an `operator` field
    // for every Java-shaped binary operator (per §3.2 table row 1).
    // Apache groups arithmetic operators across two precedence
    // levels (MUL: * / %, ADD: + -); we keep them on one rule with
    // distinct precedence numbers so the *node kind* stays uniform
    // while the precedence ladder matches Apache exactly.
    //
    // Currently covers ADDITIVE and MULTIPLICATIVE. Shift/range,
    // relational, equality, bitwise, and logical levels join this
    // rule in later iterations — each is a single line addition.
    binary_expression: $ => binaryOperators($, $._expression),

    // The last closure parameter's default may end in a name before the
    // closure's `->`, `{ a, e = x + g -> e }`, as Groovy reads it: the prec on
    // the name outranks a lambda's `g -> e` (see _closure_parameter_name_default).
    _closure_default_binary: $ => binaryOperators($, choice(
      prec(PREC.ASSIGN + 1, $.identifier),
      // a right operand that itself ends in the name: `x + y * g -> e`
      alias($._closure_default_binary, $.binary_expression),
    )),

    // §5.8 — closure expression. `{ -> }` is the explicit no-parameter
    // form; `{ a, b -> body }` has identifier parameters;
    // `{ String a, int b -> body }` has typed parameters; `{ body }`
    // has no `->` and an implicit `it` parameter (we do NOT emit a
    // synthetic `it` per §5.8). Default values and varargs land later.
    // The parameters may start on the line after the `{`.
    // A body that is one command call with one name argument, `}` right after
    // it on its line, is read as that call, as Groovy reads it and as for a
    // lambda's body: `{ x -> println x }`, `{ println it }`. Elsewhere
    // `foo bar` stays two statements (see command_chain).
    closure: $ => seq('{', ...closureBody($)),

    _closure_command: $ => seq(
      field('receiver', choice(
        $.identifier,
        $.field_access,
        $.safe_navigation_expression,
        $.spread_dot_expression,
      )),
      field('argument', alias($._closure_command_argument, $.identifier)),
    ),

    // The whole body a declaration with no value, when the type's last part
    // starts with a capital letter: `{ Foo bar }`, `{ x -> java.util.List l }`,
    // as Groovy reads it. `{ foo bar }` and `{ Foo.bar baz }` are command calls.
    _closure_declaration: $ => seq(
      $._closure_declaration_start,
      field('type', choice(alias($.identifier, $.type_identifier), $.qualified_type)),
      alias($._bare_declarator, $.variable_declarator),
    ),

    // prec.dynamic: each parameter before a `,` outweighs a declaration and a
    // lambda, `{ Map<K, V> m, int n -> }` not `Map<K, V> m, int` and `n -> `,
    // as Groovy 2.4, 3 and 4 read it. 2, not 1: a default value costs its
    // parameter 1, and the declaration's reading has defaults only before a `,`.
    closure_parameters: $ => seq(
      optional(seq(
        repeat(prec.dynamic(2, seq($.closure_parameter, ','))),
        choice(
          $.closure_parameter,
          alias($._closure_parameter_name_default, $.closure_parameter),
        ),
      )),
      '->',
    ),

    // The last parameter with a name as its default, `{ a, List<String> e = g -> e }`:
    // the `->` after the name is the closure's, not a lambda's `g -> e`, as Groovy
    // reads it in any position. The prec on the name outranks the lambda's, and only
    // with `->` next, so `{ String e = g, f = h }` stays a declaration.
    _closure_parameter_name_default: $ => seq(
      optional('final'),
      optional(choice('def', field('type', choice($._type, $.varargs_type)))),
      field('name', $.identifier),
      '=',
      field('default', choice(
        prec(PREC.ASSIGN + 1, $.identifier),
        alias($._closure_default_binary, $.binary_expression),
      )),
    ),

    // prec: in `{ x -> x }` the `x` is the closure's parameter, not the
    // parameter of a lambda in the closure body.
    // `def` may stand in for the type: `{ def m -> }`.
    // A parameter may be varargs (`String... names`) and take a default
    // value (`b = 1`), as a method's can.
    // The default-value form has no static precedence, so `{ a = 1 }` stays
    // open as a closure body holding an assignment until `->` or `}` decides.
    closure_parameter: $ => choice(
      prec(2, seq(
        repeat($.annotation),
        // `def final` too: `{ def final int x -> }`
        optional(choice('final', seq('def', 'final'))),
        optional(choice(
          seq('def', optional(field('type', choice($._type, $.varargs_type)))),
          field('type', choice($._type, $.varargs_type)),
        )),
        // `as` is a name too (Groovy 3 and 4): `{ String as -> as }`
        field('name', choice($.identifier, alias('as', $.identifier))),
      )),
      // `in` too, after a type (Groovy 3 and 4): `{ InputStream in -> }`. Only
      // after a type, where `in` is already a keyword token, so `{ in.x }`
      // still lexes `in` as a name.
      prec(2, seq(
        repeat($.annotation),
        optional(choice('final', seq('def', 'final'))),
        field('type', choice($._type, $.varargs_type)),
        field('name', alias('in', $.identifier)),
      )),
      prec.dynamic(-1, seq(
        optional('final'),
        optional(choice('def', field('type', choice($._type, $.varargs_type)))),
        field('name', choice($.identifier, alias('as', $.identifier))),
        '=',
        field('default', choice(
          $._expression,
          alias($._closure_default_call, $.method_invocation),
        )),
      )),
    ),

    // A call as the default with `->` right after its `)`, `{ File f = d() -> }`:
    // where a declaration may start, the scanner takes that `(` for a lambda's
    // (`_command_lambda_open`, as in `File f = d () -> 1`), but Groovy 2.4, 3 and 4
    // read the closure's parameter. prec.dynamic: it outweighs that reading.
    _closure_default_call: $ => prec.dynamic(3, seq(
      field('function', choice($.identifier, $.field_access)),
      field('arguments', alias($._closure_default_arguments, $.argument_list)),
    )),

    // ponytail: empty parentheses only, the form seen in real files; `d(x) ->` would need
    // the arguments to share a conflict with a lambda's parameters.
    _closure_default_arguments: $ => seq(
      alias($._command_lambda_open, '('),
      ')',
    ),

    // §5.8 — Java-style lambda (Groovy 3+): `x -> x + 1`, `() -> 42`,
    // `(a, b) -> { a + b }`, `(String s) -> s.trim()`. The body is a
    // block or an expression. prec.dynamic: `{ x -> x }` stays a closure
    // with a parameter, not a closure holding a lambda.
    lambda_expression: $ => prec.right(PREC.ASSIGN, prec.dynamic(-1, seq(
      field('parameters', choice(
        alias($.identifier, $.formal_parameter),
        alias($._lambda_parameters, $.formal_parameters),
      )),
      '->',
      // The body may be a command call: `v -> println v` (Groovy 3+).
      field('body', choice($.block, $._value)),
    ))),

    // A lambda's `(` starts an expression, so it is the in-grammar `(`,
    // never `_call_open`.
    _lambda_parameters: $ => seq(
      '(',
      optional(seq(
        $.formal_parameter,
        repeat(seq(',', $.formal_parameter)),
      )),
      ')',
    ),

    // §3.2 PRIMARY / §5.13 — list literal with trailing-comma support.
    // Empty list is `[]`; the empty map literal is the distinct `[:]`
    // form (see map_literal). Tree-sitter splits the parse between
    // list_literal and map_literal until a `:` or `]` disambiguates;
    // the conflict declaration above keeps both alternatives alive.
    // Elements may be spread: `[*a, *b]`. Groovy 2.4 drops an empty element,
    // `[a,,b]` (see `_argument_list_body`).
    list_literal: $ => {
      const element = choice(
        $._expression,
        $.spread_arguments,
        alias($._list_line_call, $.method_invocation),
      );
      return seq(
        '[',
        optional(seq(
          element,
          repeat(seq(',', repeat(','), element)),
          optional(','),
        )),
        ']',
      );
    },

    // In a list a line break ends nothing, so an argument list on the next
    // line still calls the element, as Groovy 2.4, 3 and 4 read it:
    // `[ buildDiscarder` then `(logRotator(…)) ]`. The `(` is the in-grammar
    // one, valid after an element only here; on the element's line the
    // scanner's `_call_open` takes it as usual.
    _list_line_call: $ => prec.left(PREC.POSTFIX, seq(
      field('function', choice($._expression, alias($._list_line_call, $.method_invocation))),
      field('arguments', alias($._list_line_arguments, $.argument_list)),
    )),

    _list_line_arguments: $ => seq('(', optional($._argument_list_body), ')'),

    // §3.2 PRIMARY / §5.3 — map literal. `[:]` is the empty map;
    // anything else is one-or-more `key: value` entries. The
    // distinction between map_entry and a labelled statement is
    // contextual — labels are statement-start-only (see §5.3 and
    // the LABEL_COLON scanner branch), so inside `[ … ]` a `:` after
    // an identifier is always a map key separator.
    map_literal: $ => {
      const entry = choice($.map_entry, $.spread_map_entry);
      return seq(
        '[',
        choice(
          ':',
          seq(
            entry,
            // `[a: 1,, b: 2]`: Groovy 2.4's empty element (see `_argument_list_body`)
            repeat(seq(',', repeat(','), entry)),
            optional(','),
          ),
        ),
        ']',
      );
    },

    // A keyword that can also start an expression is a key too: `[new: 1]`,
    // `f(switch: 'on')`.
    map_entry: $ => seq(
      field('key', choice(
        $._expression,
        reserved('properties', $.identifier),
        alias('new', $.identifier),
        alias('switch', $.identifier),
      )),
      choice(':', alias($._expression_colon, ':')),
      field('value', $._expression),
    ),

    // §5.11 — `*: expr` only inside a map literal. `*:` is a single
    // anonymous token (longest-match keeps it intact); a bare `*`
    // followed by a `:` colon outside this context would have to be
    // syntactically impossible by the surrounding rules.
    spread_map_entry: $ => seq(
      choice('*:', seq('*', ':')),
      field('value', $._expression),
    ),

    // §3.2 PRIMARY — `( expr )`. Distinct from `parenthesized_type_cast`
    // (§5.15), which lands once we have `_type`. The parser disambiguates
    // by what's inside the parens.
    parenthesized_expression: $ => seq(
      '(',
      choice($._expression, alias($._parenthesized_command, $.command_chain)),
      ')',
    ),

    // §5.5 — a command call inside parentheses, as Geb page objects write it:
    // `(el searchBy).click()`, `(go LoginPage).login(u, p)`,
    // `return (run service: 'x', with: m)`. Inside `( … )` a name after the
    // receiver can't start a declaration, so it is the argument.
    // A call needs no argument of its own (Groovy 3 and 4):
    // `(f(a) intersect b)` is `f(a).intersect(b)`. A name left over at the
    // end is a property: `(a intersect b)` is `a(intersect).b`.
    _parenthesized_command: $ => seq(
      choice(
        seq(
          // also a quoted name (Groovy 3 and 4): `("foo" 1)` is `foo(1)`
          field('receiver', choice($.identifier, $.field_access, $.string_literal)),
          choice(
            seq($._command_literal_start, field('argument', choice(
              $._expression,
              alias($._command_quoted_named_argument, $.named_argument),
            ))),
            // `if (a.b c[0] - n)` is `a.b(c[0] - n)`: a name an index or a binary
            // operator follows (see `_paren_command_start`)
            seq($._paren_command_start, field('argument', $._expression)),
            field('argument', alias($._command_named_argument, $.named_argument)),
            // `(f a : 1)`: a key a space keeps from its `:`, which the scanner
            // leaves to the grammar after looking past the space
            field('argument', alias($._paren_named_argument, $.named_argument)),
            field('argument', $.identifier),
            // `if (valueOf findProperty(s, 'x'))`
            field('argument', alias($._name_call, $.method_invocation)),
          ),
          repeat($._command_later_argument),
          // `(page.optionList 0 isEmpty())`: a call chained after the argument,
          // or a command chained after it, `(1.plus 2 plus 3)` (`1.plus(2).plus(3)`).
          repeat($._parenthesized_command_link),
        ),
        seq(
          field('receiver', $.method_invocation),
          repeat1($._parenthesized_command_link),
        ),
        // `(Integer.parseInt(s), 16)` is `Integer.parseInt((s), 16)` (Groovy 3
        // and 4): the parenthesized first argument that the comma tells from
        // an argument list, as for `assertEquals (-1), x`.
        seq(
          field('receiver', choice($.identifier, $.field_access)),
          field('argument', alias($._command_parenthesized_argument, $.parenthesized_expression)),
          repeat1($._command_later_argument),
        ),
      ),
      // also a quoted one: `("a" 1 "b")` is `a(1).b`
      optional(choice($.identifier, $.string_literal)),
    ),

    _paren_named_argument: $ => seq(
      field('key', $.identifier),
      ':',
      field('value', $._expression),
    ),

    _parenthesized_command_link: $ => choice(
      field('argument', alias($._name_call, $.method_invocation)),
      seq($.identifier, field('argument', $._expression), repeat($._command_later_argument)),
    ),

    // The last call may take a closure: `or (b) or (c) { }`.
    _name_call: $ => prec.right(seq(
      field('function', $.identifier),
      field('arguments', $.argument_list),
      optional(field('closure', $.closure)),
    )),

    // §3.2 — single `unary_expression` node kind but two precedence
    // alternatives, matching Apache §3.1:
    //   UNARY_NOT (`!` `~`)  — level 16, tighter than POWER (15),
    //                          so `~2 ** 3` parses as `(~2) ** 3`.
    //   UNARY     (`+` `-`)  — level 14, looser than POWER (15),
    //                          so `-2 ** 3` parses as `-(2 ** 3)`.
    // Both alternatives produce the same node kind, so downstream
    // tools see one consistent shape and discriminate by operator.
    unary_expression: $ => choice(
      prec.right(PREC.UNARY_NOT, seq(
        field('operator', choice('!', '~')),
        field('operand', $._expression),
      )),
      prec.right(PREC.UNARY, seq(
        field('operator', choice('+', '-')),
        field('operand', $._expression),
      )),
    ),

    // §3.2 PRIMARY — boolean and null are keywords in Groovy. The
    // `word: $.identifier` directive above lets tree-sitter resolve
    // the keyword-vs-identifier ambiguity automatically: a literal
    // 'true' / 'false' / 'null' that matches the identifier regex
    // is reclassified as the keyword token when one of these rules
    // is the only legal next reduction.
    boolean_literal: _ => choice('true', 'false'),

    null_literal: _ => 'null',

    // §5.7 — string flavours. Single and triple-single never
    // interpolate. Double-quoted and triple-double interpolate
    // `$identifier(.identifier)*` and `${expression}` as structured
    // children, so consumers can walk into the GString's expression
    // segments. Slashy bodies and dollar-slashy bodies remain flat
    // tokens (their internal `$` is rare in practice and the
    // scanner cannot interrupt a slashy mid-body without per-flavor
    // state).
    string_literal: $ => choice(
      $._single_quoted_string,
      $._triple_single_quoted_string,
      $._double_quoted_string,
      $._triple_double_quoted_string,
      $._dollar_slashy_string,
      $._slashy_string,
    ),

    // §5.7 — slashy regex `/.../`. The opening `/` is emitted by
    // the external scanner (after the context guard from §5.4 / §6.2
    // and a lookahead that confirms a closing `/` exists on the same
    // line). The body is parsed as `gstringPart` so `$identifier` and
    // `${expr}` segments expose the same structured children as
    // double-quoted GStrings. The closing `/` is `token.immediate`
    // so it cannot skip whitespace between the body and the closer.
    _slashy_string: $ => seq(
      $._slashy_string_start,
      repeat(gstringPart($, $._slashy_text)),
      token.immediate('/'),
    ),

    // `\/` is the one escape in a slashy string; any other backslash is a
    // character of its own, so `:\\/\\/` holds two escaped slashes and
    // doesn't end the string at the first `/`. `\$` stays a pair, so the
    // `$` after it starts no interpolation.
    _slashy_text: _ => token.immediate(repeat1(choice(
      /[^\/\\$]/,
      /\\[\/$]/,
      /\\/,
    ))),

    _single_quoted_string: _ => token(seq(
      '\'',
      repeat(choice(
        /[^'\\\n\r]/,
        // a line continuation; `\[\s\S]` alone would stop at a CRLF's `\n`
        /\\\r\n/,
        /\\[\s\S]/,
      )),
      '\'',
    )),

    _triple_single_quoted_string: _ => token(seq(
      '\'\'\'',
      repeat(choice(
        /[^'\\]/,
        /\\[\s\S]/,
        /'[^']/,
        /''[^']/,
      )),
      // the content may end with one or two quotes: `'''a='b''''`
      /''''?'?/,
    )),

    // §5.7 / §6.3 — double-quoted GString. The opening `"` is
    // tokenised separately from the body so that `$identifier` and
    // `${expr}` segments parse as proper child nodes. The text
    // fragment stops before any `$`, so `$identifier` and `${expr}`
    // get a chance to match; literal `$` (followed by `"` or a
    // digit etc.) is recovered as `$` plus the fallback shape.
    _double_quoted_string: $ => seq(
      '"',
      repeat(gstringPart($, $._gstring_text)),
      token.immediate('"'),
    ),

    _gstring_text: _ => token.immediate(repeat1(choice(
      /[^"\\$\n\r]/,
      /\\\r\n/,
      /\\[\s\S]/,
    ))),

    // Bare `$` inside a GString body that does not start a valid
    // interpolation (`$"`, `$5`, `$$`, `$ `) is matched as a one-
    // char literal fragment. This rule must have lower precedence
    // than `_gstring_dollar_path` so `$name` still wins.
    _gstring_literal_dollar: _ => token.immediate(prec(-1, /\$/)),

    // §5.7 — triple-double GString. Like `_double_quoted_string`
    // but newlines and embedded `"` / `""` are part of the body.
    _triple_double_quoted_string: $ => seq(
      '"""',
      repeat(gstringPart($, $._triple_gstring_text)),
      // the content may end with one or two quotes: `"""say "hi""""`
      alias(token.immediate(/""""?"?/), '"""'),
    ),

    _triple_gstring_text: _ => token.immediate(repeat1(choice(
      /[^"\\$]/,
      /\\[\s\S]/,
      /"[^"\\]/,
      /""[^"\\]/,
      /"\\[\s\S]/,
      /""\\[\s\S]/,
    ))),

    // §5.7 — `$identifier(.identifier)*` interpolation. The whole
    // `$identifier...` shape is matched as a single immediate token
    // so the `$` only consumes when followed by an identifier start;
    // `_gstring_literal_dollar` handles the lone-`$` fallback above.
    gstring_dollar_interpolation: $ => seq(
      field('value', alias(
        $._gstring_dollar_path,
        $.identifier,
      )),
    ),

    _gstring_dollar_path: _ => token.immediate(
      /\$[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*/,
    ),

    // §5.7 / §6.3 — `${expression}` interpolation. The body is a closure
    // body: statements (`${x.replace 'a', 'b'}`, `${if (k) { 'a' } else
    // { 'b' }}`, `${n += 1; n}`, `${assert ok; x}`, labels as in
    // `${group:name:version}`) and the lazy form `${-> x}`.
    gstring_brace_interpolation: $ => choice(
      prec(1, seq(token.immediate('${'), field('value', $._expression), '}')),
      seq(
        token.immediate('${'),
        optional($.closure_parameters),
        repeat(choice(
          $.expression_statement,
          $.command_chain,
          $.if_statement,
          $.local_variable_declaration,
          $.for_statement,
          $.for_in_statement,
          $.while_statement,
          $.do_while_statement,
          $.try_statement,
          $.assert_statement,
          $.labeled_statement,
          // `${-> x; return y}`
          $.return_statement,
          ';',
          $._line_break,
        )),
        '}',
      ),
    ),

    // §5.7 — `$/...$/` dollar-slashy strings. The body is parsed
    // through `gstringPart` so `$identifier` and `${expr}` segments
    // expose structured children. `_dollar_slashy_text` consumes
    // every literal-text alternative: `/X` where X is not `$`,
    // `$$` (literal `$`), `$/` (literal `/`), and any other
    // non-`/` non-`$` char. The closer `/$` is `token.immediate`
    // so it always wins over text when `/` is followed by `$`.
    _dollar_slashy_string: $ => seq(
      '$/',
      repeat(gstringPart($, $._dollar_slashy_text)),
      // A `/` before the `/$` is text: `$/a//$` is `a/`. The text can't hold
      // that `/` without lookahead, so the closer takes the run of slashes.
      alias(token.immediate(/\/+\$/), '/$'),
    ),

    _dollar_slashy_text: _ => token.immediate(repeat1(choice(
      /[^/$]/,
      /\/+[^/$]/,
      /\$\$/,
      /\$\//,
    ))),

    // §5.1 — hex (0x…), binary (0b…), decimal int / float / scientific,
    // each with optional type suffix from { G g L l I i D d F f }.
    // Underscores between digits are accepted permissively (the spec
    // forbids leading/trailing `_`, but that is a semantic check,
    // not a grammar one — see §1.2).
    number_literal: _ => token(choice(
      /0[xX][0-9a-fA-F_]+[GgLlIi]?/,
      /0[bB][01_]+[GgLlIi]?/,
      /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?[GgFfDd]?/,
      // no digits before the point: `.5`, `.05D`
      /\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?[GgFfDd]?/,
      /[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*[GgFfDd]?/,
      /[0-9][0-9_]*[GgLlIiDdFf]?/,
    )),

    identifier: _ => /[\p{L}_$][\p{L}\p{Nd}_$]*/,
  },
});
