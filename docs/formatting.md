# Formatting

## What formatting changes

By default, formatting changes whitespace only:

- Every line is re-indented from the parse tree (blocks, closures, argument lists, list and map literals, continued expressions, Spock `when:` / `then:` blocks).
- Trailing whitespace is removed, runs of blank lines collapse to one, and the file ends with one newline.
- Lines inside multi-line strings and comments are left exactly as they are.

Two config options do more, and both are off by default. [`normalize_spacing`](#spacing-within-lines) fixes the spaces between tokens within a line. [`max_line_length`](#line-wrapping) also adds line breaks.

## Why it is safe

The output is parsed again and its token stream compared with the input's. If anything other than whitespace would change, or the input doesn't parse cleanly, the file is left untouched (`-v` lists these). Running it twice changes nothing the second time.

In-place writes go to a temporary file beside the original, which is then renamed over it, so a crash never leaves half a file. Symlinks are handled as described in [command line](usage.md#symlinks). [Windows](windows.md) has its own details.

## Spacing within lines

With `normalize_spacing = true` in the config, groovyfmt also fixes the spaces between tokens on each line. It never adds or removes a line break and never touches strings, GString interpolations or comments.

### The style

- one space around binary operators, assignment (`=`, `+=`), `?` and `:` in a ternary, `?:`, `->`, `**`, `as`, `in` and `instanceof`
- one space after a comma and none before it; none before a `;`, one after it inside a `for (…)`
- no space inside `(…)` or `[…]`, and none inside `<…>` of a generic type (`Map<String, Integer>`)
- no space between a call, constructor, annotation or method declaration and its `(` (`foo(x)`, `new Foo()`, `@Ann(x)`, `def f(a)`), one after `if`, `for`, `while`, `switch`, `catch` and `synchronized`
- one space before a `{`, inside a one-line `{ … }` and around `else`, `catch` and `finally` after a `}`
- in map entries, named arguments, labels and `case x:`, no space before the `:`, and one after it in map entries and named arguments
- no space after a prefix operator (`-x`, `!x`, `*args`) or before a postfix one (`i++`), around `::`, or before the `[` of an array type (`String[]`), and one after a cast (`(String) x`)
- any other run of spaces becomes one

### Left as written

Some spacing is left as written because changing it could change what the code means or how it reads:

- member access (`.`, `?.`, `*.`, `.&`)
- ranges (`1..n`)
- a subscript after a space (`a [1]`)
- a call with a space before `(` whose result is used further (`foo (x).bar()`, `foo (x) + 1`), which Groovy may read as a command call
- a binary operator with a space before it and none after (`a -b`, `println -1`)
- `<` and `>` comparisons chained like a generic type (`x as Kind<Path>[]`)
- a closure parameter's default (`{ int b=2 -> }`)
- closures that look like template placeholders (`{{name}}`, `${x}` outside a string, a closure standing alone as a statement)

### Alignment

Alignment is kept.

- A run of two or more spaces, or a tab, stays when the token after it lines up with a token on a nearby line. A nearby line is the line above or below, or a line indented the same way within 20 lines. This covers an `=` column, map keys and values across nested values, Gradle dependency columns and trailing comments.
- A single space before the `:` of a map key or named argument stays only when a nearby line has the `:` of a key of a different length in the same column, so that the shorter key was padded to line the colons up (`9 :` over `10:`). Keys of the same length (`access :` over `access :`, or over an `access:` that starts a column later) line up only by coincidence, so the space goes.
- When something earlier on the line gets shorter or longer, a run before a trailing comment absorbs the difference, so the comment stays lined up with one on a nearby line that doesn't move. When the lines around it move the same way, the comment moves with them.
- On a line laid out in columns (`[ 4,  4,  3]` over `[10,  9,  8]`), nothing before its last aligned column changes.
- A line whose changes would move a `=`, `:` or trailing comment out of line with the same token on a nearby line keeps its spacing up to that token, unless the nearby line would move it the same way.
- Spock data tables are never touched. Every statement from a `where:` label to the end of its block keeps its spacing exactly.
- A gap next to a comment is kept.

Each of these decisions is made from the text the lines have when respacing finishes, never from how they were written before. So formatting the output again changes nothing, and a copy of it with a few spaces or lines edited is settled by one run.

### Checks and limits

Besides the token check, spacing output must parse to the same tree shape as the input. If it doesn't, the file is formatted without spacing changes instead.

Limits, so a hostile file can't make it slow:

- A line longer than 1,000 bytes keeps its spacing (it is still re-indented).
- Alignment is looked up only in the first 2,000 characters of a line.
- Lines whose alignment depends on each other are settled in repeated passes over the file, each linear in its size. Two to four passes are typical. A file that hasn't settled after 64 is skipped (`-v` lists it as `spacing did not settle`) rather than written half-respaced.

## Line wrapping

With `max_line_length = <n>` in the config, groovyfmt breaks lines longer than `n` columns where that is safe. `n` is 40 to 400. There is no default, and without the key nothing is wrapped.

A tab counts as `indent` columns, and width is counted in characters (not bytes). Wrapping only ever adds line breaks. It never joins lines, never breaks inside a token, and never touches a line inside a string or comment.

### Where a line breaks

In Groovy a line break can end a statement. So a break goes only where Groovy's own grammar (the Parrot parser's `GroovyLexer.g4` and `GroovyParser.g4`) reads the code the same with or without it:

- **Inside `(…)` and `[…]`**, where Groovy's lexer drops newlines before the parser sees them. An argument list, list literal or map literal with two or more items breaks one item per line, after the opening bracket and each comma, with the closing bracket on a line of its own. A trailing closure stays on the line of the `)` (`) {`). The same goes for:
  - a method's or constructor's parameter list with two or more parameters (`def f(` then one parameter per line, then `) {`)
  - an annotation's arguments, and a list in them (`@Ann(` then `name = value` per line, then `)`), when nothing follows the annotation on its line
  - a call's only argument, or a list's only item, after the opening bracket and before the closing one (`throw new IllegalStateException(` then the message, then `)`). Only when the argument is at least 20 columns and at least half as wide as what comes before it on the line, when it then fits on its line (or is an operator expression or a chain that breaks further), when only closing brackets, `,` or `;` follow it on its line, and when an `=` before it can't break instead (`def x =` then the whole call reads better than `def x = a.b(` then a short argument). A list, map or closure as the only argument, or a call ending in arguments that break that way (`f(g(a, b))`, `f(g(h([a, b])))`), breaks inside itself instead (`f(g(` then `a` and `b`, then `))`).
- **After each comma of a parenthesis-free command call's arguments** (`implementation group: 'a',` then `name: 'b'` on the next line), when nothing but a comment follows the call on its line and the line up to the first comma then fits. Groovy's grammar takes a newline after each of them (`argumentList : firstArgumentListElement (COMMA nls argumentListElement)*`).
- **Before each `.`, `?.` or `*.` of a chain of three or more calls**, except the first call's (`Foo.builder()`, then `.name(x)` and `.build()` on lines of their own). The grammar allows this anywhere (`pathElement : NL* DOT …`). In a chain already written over several lines, the dots on a line that is still too long break the same way.
- **After `||`, `&&`, `==`, `!=`, `+` and `-`** at the lowest-precedence level of a binary expression, after a ternary's `?` and `:`, and after `?:`. The operator always ends the line. The grammar takes a newline after `+` and `-` but not before them, and `a` then `-b` on the next line would be two statements.
- **After `=`** in an assignment or declaration, when nothing else on the line can break.

For each line over the limit, the outermost construct that lies wholly on that line breaks one of these ways. The file is then re-indented (and respaced, with `normalize_spacing`), the lines are measured again, and pieces still too long break further. Continuation lines get `continuation_indent`.

An operator or `=` breaks only when every resulting piece fits. So `a.b().c().d() == x` breaks the chain, and `x == [a, b, c]` breaks the list instead. A single binary operator doesn't leave a piece shorter than 8 columns (`f(x) ==` then `1`).

### Never broken

These are never broken, because Groovy or the grammar might read the result differently, or because the result would read worse than the long line:

- In a parenthesis-free command call, anything but the commas between its arguments: between the name and the first argument, inside an argument (`input 'x', options: [a, b]` keeps `[a, b]` whole, `println a + b` its `+`), and in a command call inside another's arguments or followed on its line by more than a comment. tree-sitter-groovy finds a command call's first argument by looking at line ends. Also a statement that starts on the line where the one before it ends without a `;`, which is how the grammar reads some command calls (`println a + b` as `println` and `a + b`).
- Anything inside a closure or block written on one line (`list.each { foo(a, b) }`), whose statements end at line breaks.
- Between `return`, `throw`, `assert`, `new`, `as`, `in`, `instanceof` or `!` and what follows, between a name and its `(`, between `)` and a trailing closure, and anywhere the line before the break could end a statement by itself.
- Outside `(…)` and `[…]`, a wrapped line never starts with `{`, `++`, `--`, `+`, `-`, `*` or `/` (a `*.` of a chain aside). It starts with `(`, `[` or a string only right after a line ending in a binary operator, `?`, `:` or `=`, which the grammar says must continue. Inside brackets Groovy's lexer drops the line break, so an argument such as `-1` or `'text'` may start a line.
- In an annotation, anything but its brackets: its argument list and the lists in it (an operator or a chain in them stays whole), and an annotation followed on its line by more (`@A(x, y) def f()`). `import` and `package` lines, `case` labels, Spock `where:` blocks, GString interpolations, and argument lists, parameter lists or literals holding a comment.
- `<`, `>`, `as`, `instanceof`, `in`, ranges, shifts and the other operators. Only the ones above are break points. Nothing breaks right after a `>` either, since tree-sitter-groovy reads `f(a, List<T>` followed by a line break as comparisons.
- A part of a larger construct on the line that can't break itself:
  - an operator expression inside another operator's operand (`(a != null && b) ? x : y`), in parentheses (`(a ?: b).c()`) or in a subscript (`a[i + 1]`)
  - an operator expression or chain that is a call's only argument while the call is on one line (`f(a + b)`, also through a cast: the parentheses break first) or a lambda's body
  - anything in a one-line ternary's condition or first branch (`c ? f(a, b) : d`)
  - the only argument written `key: value`, while the call is on one line
  - the condition of an `assert` with a message, or of an `if` or `while` whose body follows on the same line (braces or not)
  - whatever is inside something that may not break
- An argument list or literal followed on its line by more text than it holds (`f(_, _) >> { long closure parameters ->`), where breaking it gains little. A call's only argument followed by more than closing brackets (`f(x) == 1`, `if (f(x)) {`), or shorter than half of what comes before it on the line (`a.b(c).d(x)`), for the same reason.
- A parameter list of a closure or lambda, and one with a single parameter.
- A line holding two statements (`a(x, y); b(z)`), or an `if`, `else`, `while` or `for` body without braces (`if (c) x = f(a, b)`).
- A line of a list written over several lines that holds two or more of its items (laid out by hand).
- A line whose overflow is only a trailing comment, or that holds a string too long to fit on a continuation line anyway.

A line that is still too long after every safe break stays as it is.

### Checks and limits

- Each pass parses the result again and must find the same tree shape. The final text must have the same tokens and tree shape as the input, or the file is formatted without wrapping instead.
- Formatting the output again changes nothing, for any combination of the other settings.
- Wrapping goes at most 16 constructs deep on one line, and isn't tried on a file nested more than 1,000 levels deep. Such files are formatted without wrapping.
- Lines between `// groovyfmt: off` and `// groovyfmt: on` are never wrapped.
- With `--lines`, only lines in the ranges are wrapped, and a wrapped line's pieces take its place, so the lines after it move down.
- `-v` lists each file formatted without wrapping and why (`not wrapped <file>: <reason>`).
- `--suggest-style` ignores `max_line_length`. It measures indent styles only.

## Turning formatting off

The lines from `// groovyfmt: off` to `// groovyfmt: on` come out byte for byte as they were. Both comments are included, and a missing `on` runs to the end of the file. The rest of the file is formatted as with [`--lines`](usage.md#formatting-some-lines), so in a file with an `off` comment no line is added or removed.

Lint has its own suppression comments ([lint](lint.md#suppressing-reports)).
