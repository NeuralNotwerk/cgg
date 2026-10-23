//! Same-length rewrites that let tree-sitter-groovy parse common Groovy
//! shapes it rejects (WS38). The grammar reads the rewritten copy;
//! extraction reads the original bytes, so every node's text is what the
//! file says and every byte offset and line is unchanged.
//!
//! Each rewrite replaces bytes of a span it has recognised in code — never
//! inside a string's content, a GString interpolation or a comment — with
//! spaces, a digit, `+` or a quote, and never touches a newline:
//!
//! - safe navigation and spread-dot, `a?.b` / `a*.b`: the `?` or `*`
//!   becomes a space (`a .b`), a member access the grammar reads;
//! - a Spock interaction's wildcard count, `_ * m.call(_)` at the start of
//!   a line: `_` becomes `0`;
//! - a spread argument or list element, `f(*args)` / `[*xs]`: the `*`
//!   becomes a space;
//! - an `as` coercion, `x as Type`, `(x as Foo).m()`, `g([] as Set)`: the
//!   `as` and its type become spaces (`import a.B as C` is left alone);
//! - the empty map `[:]`: `[ ]`;
//! - `for (def x in xs)`: the `def` becomes spaces;
//! - a command expression the grammar rejects, `run "x"` in a method body
//!   of a type or `compileSdk rootProject.ext.v` anywhere: the blank after
//!   its name and its top-level commas become `+`, a binary expression;
//!
//! and, changing a string's delimiters but never its content:
//!
//! - the empty string `''`: `""`;
//! - a one-line slashy string `/a.b/`: `"a.b"` (or `'a.b'` when it holds a
//!   `"`).
//!
//! A line where the scanner cannot tell a slashy string from a division
//! gets no rewrite after that point.

/// Lexical states of the scanner, innermost last.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    /// Code inside a GString's `${ … }`, with its brace depth: inside a
    /// string, so never rewritten.
    Interp(u32),
    LineComment,
    BlockComment,
    /// `'…'`, `"…"`, `'''…'''`, `"""…"""`: the quote byte and whether
    /// the string is triple-quoted.
    Str(u8, bool),
    /// `/…/`, from the byte offset of its opening slash; whether it
    /// opened in code (not inside a GString's `${ … }`).
    Slashy(usize, bool),
    /// `$/…/$`.
    DollarSlashy,
}

fn ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// The rewritten copy of `src`, or `None` when no rewrite applies.
pub(crate) fn rewrite(src: &[u8]) -> Option<Vec<u8>> {
    let mut out: Option<Vec<u8>> = None;
    let mut set = |out: &mut Option<Vec<u8>>, at: usize, b: u8| {
        out.get_or_insert_with(|| src.to_vec())[at] = b;
    };
    let mut stack: Vec<State> = vec![State::Code];
    // The last significant (non-blank) code byte, for telling a slashy
    // string from a division.
    let mut prev_sig: u8 = b'\n';
    // Set when a `/` could not be classified: no rewrite until the line
    // ends.
    let mut unsure_line = false;
    // Whether the current line is an `import` (its `as` is an alias).
    let mut import_line = false;
    let mut line_start = true;
    // What each open `{` in code opens, innermost last.
    let mut braces: Vec<Brace> = Vec::new();
    // Where the innermost open method body began (its `{`), for a body
    // written on one line.
    let mut method_open: Vec<usize> = Vec::new();
    // A `class`/`interface`/`enum`/`trait` header whose `{` is next.
    let mut type_pending = false;
    // Open `(`, `[` and `{` in code.
    let mut nest = 0u32;
    // A command expression being rewritten (see [`command_at`]): its
    // nesting level, whose commas become `+`.
    let mut command: Option<u32> = None;
    // A closure's `->` was the last code token: its body's first statement
    // is next.
    let mut after_arrow = false;
    let n = src.len();
    let mut i = 0usize;
    while i < n {
        let b = src[i];
        let state = *stack.last().unwrap_or(&State::Code);
        if b == b'\n' {
            if matches!(state, State::Code | State::Interp(_) | State::LineComment) {
                unsure_line = false;
                import_line = false;
                line_start = true;
                // A statement ends at a line break, unless its last
                // argument is followed by a comma.
                if state != State::Interp(0) && command == Some(nest) && prev_sig != b','
                {
                    command = None;
                }
            }
            if state == State::LineComment {
                stack.pop();
            }
            i += 1;
            continue;
        }
        match state {
            State::LineComment => {
                i += 1;
            }
            State::BlockComment => {
                if b == b'*' && src.get(i + 1) == Some(&b'/') {
                    stack.pop();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            State::Str(q, triple) => {
                if b == b'\\' {
                    i += 2;
                } else if b == b'$' && q == b'"' && src.get(i + 1) == Some(&b'{') {
                    stack.push(State::Interp(0));
                    i += 2;
                } else if b == q {
                    if !triple {
                        stack.pop();
                        i += 1;
                    } else if src.get(i + 1) == Some(&q) && src.get(i + 2) == Some(&q) {
                        stack.pop();
                        i += 3;
                    } else {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            State::Slashy(open, in_code) => {
                if b == b'\\' {
                    i += 2;
                } else if b == b'$' && src.get(i + 1) == Some(&b'{') {
                    stack.push(State::Interp(0));
                    i += 2;
                } else if b == b'/' {
                    stack.pop();
                    // The grammar has no slashy string: a one-line one is
                    // quoted instead, its content untouched, with a quote
                    // it does not contain.
                    let content = &src[open + 1..i];
                    if in_code && !content.contains(&b'\n') {
                        let q = if !content.contains(&b'"') {
                            Some(b'"')
                        } else if !content.contains(&b'\'') && !content.contains(&b'$') {
                            Some(b'\'')
                        } else {
                            None
                        };
                        if let Some(q) = q {
                            set(&mut out, open, q);
                            set(&mut out, i, q);
                        }
                    }
                    prev_sig = b'"';
                    i += 1;
                } else {
                    i += 1;
                }
            }
            State::DollarSlashy => {
                if b == b'$' && src.get(i + 1) == Some(&b'{') {
                    stack.push(State::Interp(0));
                    i += 2;
                } else if b == b'$' {
                    // `$$` and `$/` are escapes.
                    i += 2;
                } else if b == b'/' && src.get(i + 1) == Some(&b'$') {
                    stack.pop();
                    prev_sig = b'"';
                    i += 2;
                } else {
                    i += 1;
                }
            }
            State::Code | State::Interp(_) => {
                let in_code = state == State::Code;
                // Strings and comments open the same way in both.
                if b == b'/' && src.get(i + 1) == Some(&b'/') {
                    stack.push(State::LineComment);
                    i += 2;
                    continue;
                }
                if b == b'/' && src.get(i + 1) == Some(&b'*') {
                    stack.push(State::BlockComment);
                    i += 2;
                    continue;
                }
                // `''`, the empty single-quoted string, which the grammar
                // rejects: `""` (no content to keep).
                if b == b'\''
                    && src.get(i + 1) == Some(&b'\'')
                    && src.get(i + 2) != Some(&b'\'')
                {
                    if in_code && !unsure_line {
                        set(&mut out, i, b'"');
                        set(&mut out, i + 1, b'"');
                    }
                    prev_sig = b'"';
                    line_start = false;
                    i += 2;
                    continue;
                }
                if b == b'\'' || b == b'"' {
                    let triple = src.get(i + 1) == Some(&b) && src.get(i + 2) == Some(&b);
                    stack.push(State::Str(b, triple));
                    prev_sig = b'"';
                    line_start = false;
                    i += if triple { 3 } else { 1 };
                    continue;
                }
                if b == b'$' && src.get(i + 1) == Some(&b'/') {
                    stack.push(State::DollarSlashy);
                    line_start = false;
                    i += 2;
                    continue;
                }
                if b == b'/' {
                    // A slashy string follows an operator or an opening
                    // bracket; a division follows a value. Anything else
                    // is unsure.
                    if matches!(
                        prev_sig,
                        b'(' | b','
                            | b'='
                            | b'['
                            | b'{'
                            | b':'
                            | b'!'
                            | b'&'
                            | b'|'
                            | b'?'
                            | b';'
                            | b'~'
                            | b'\n'
                    ) {
                        stack.push(State::Slashy(i, in_code && !unsure_line));
                        line_start = false;
                        i += 1;
                        continue;
                    }
                    if !(ident_byte(prev_sig) || matches!(prev_sig, b')' | b']' | b'"')) {
                        unsure_line = true;
                    }
                    prev_sig = b'/';
                    i += 1;
                    continue;
                }
                if let State::Interp(depth) = state {
                    if b == b'{' {
                        *stack.last_mut().unwrap() = State::Interp(depth + 1);
                    } else if b == b'}' {
                        if depth == 0 {
                            stack.pop();
                        } else {
                            *stack.last_mut().unwrap() = State::Interp(depth - 1);
                        }
                    }
                }
                if in_code {
                    match b {
                        b'(' | b'[' => nest += 1,
                        b')' | b']' => nest = nest.saturating_sub(1),
                        b'{' => {
                            let kind = if type_pending {
                                Brace::TypeBody
                            } else if braces.last() == Some(&Brace::TypeBody) {
                                Brace::MethodBody
                            } else {
                                Brace::Other
                            };
                            if kind == Brace::MethodBody {
                                method_open.push(i);
                            }
                            braces.push(kind);
                            type_pending = false;
                            nest += 1;
                        }
                        b'}' => {
                            // A method body on one line, `{ x.y() }`, needs
                            // its statement ended: the blank before `}`
                            // becomes `;`.
                            if braces.last() == Some(&Brace::MethodBody)
                                && let Some(open) = method_open.pop()
                                && !unsure_line
                                && !src[open..i].contains(&b'\n')
                                // Method pointers keep the shape the
                                // plugin reads them from.
                                && !src[open..i].windows(2).any(|w| w == b".&")
                                && i > 0
                                && matches!(src[i - 1], b' ' | b'\t')
                                && src[open + 1..i].iter().rposition(|c| !c.is_ascii_whitespace()).is_some_and(|p| {
                                    !matches!(src[open + 1 + p], b';' | b'{' | b'}')
                                })
                            {
                                set(&mut out, i - 1, b';');
                            }
                            braces.pop();
                            nest = nest.saturating_sub(1);
                        }
                        b';' => {
                            if command == Some(nest) {
                                command = None;
                            }
                        }
                        b',' if command == Some(nest) && !unsure_line => {
                            set(&mut out, i, b'+');
                            rewrite_named_key(src, i + 1, &mut out, &mut set);
                        }
                        _ => {}
                    }
                }
                if in_code && b == b'-' && src.get(i + 1) == Some(&b'>') {
                    after_arrow = true;
                    prev_sig = b'>';
                    line_start = false;
                    i += 2;
                    continue;
                }
                // A closure whose body opens with a declaration
                // (`{ x ->\n def d = 1`) is an ERROR: the declaration's
                // head (`def`, a type) becomes blanks, an assignment.
                if after_arrow && !b.is_ascii_whitespace() {
                    after_arrow = false;
                    if in_code
                        && !unsure_line
                        && let Some(end) = declaration_head_end(src, i)
                    {
                        for at in i..end {
                            set(&mut out, at, b' ');
                        }
                        line_start = false;
                        prev_sig = b' ';
                        i = end;
                        continue;
                    }
                }
                if in_code && !unsure_line {
                    if line_start && b.is_ascii_alphabetic() {
                        import_line = src[i..].starts_with(b"import")
                            && src.get(i + 6).is_some_and(|c| c.is_ascii_whitespace());
                    }
                    // A command the grammar rejects: `run "x"` -> `run+"x"`
                    // directly in a method body of a type; `compileSdk
                    // rootProject.ext.v`, `succeeds tasks()` anywhere but a
                    // type body.
                    if line_start
                        && command.is_none()
                        && braces.last() != Some(&Brace::TypeBody)
                        && let Some(gap) =
                            command_at(src, i, braces.last() == Some(&Brace::MethodBody))
                    {
                        set(&mut out, gap, b'+');
                        rewrite_named_key(src, gap, &mut out, &mut set);
                        command = Some(nest);
                        line_start = false;
                        prev_sig = b'+';
                        i = gap + 1;
                        continue;
                    }
                    if let Some(len) = rewrite_at(
                        src,
                        i,
                        prev_sig,
                        line_start,
                        import_line,
                        &mut out,
                        &mut set,
                    ) {
                        if !b.is_ascii_whitespace() {
                            line_start = false;
                        }
                        prev_sig = src[i + len - 1];
                        i += len;
                        continue;
                    }
                }
                if !b.is_ascii_whitespace() {
                    prev_sig = b;
                    line_start = false;
                }
                // A word that ends in an operator keyword reads as an
                // operator for the slashy test (`return /x/`).
                if ident_start(b) {
                    let end = src[i..]
                        .iter()
                        .position(|c| !ident_byte(*c))
                        .map_or(n, |p| i + p);
                    let word = &src[i..end];
                    // A type declaration's header: its `{` opens a type
                    // body (not `Foo.class`, nor a `class:` map key).
                    if in_code
                        && matches!(word, b"class" | b"interface" | b"enum" | b"trait")
                        && !matches!(prev_sig, b'.')
                        && src[end..].iter().find(|c| !c.is_ascii_whitespace())
                            != Some(&b':')
                    {
                        type_pending = true;
                    }
                    prev_sig = if matches!(word, b"return" | b"assert" | b"in" | b"case")
                    {
                        b'='
                    } else {
                        b'a'
                    };
                    line_start = false;
                    i = end;
                    continue;
                }
                i += 1;
            }
        }
    }
    out
}

/// What a `{` in code opens.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Brace {
    /// A class, interface, enum or trait body.
    TypeBody,
    /// A block directly in a type body: a method, constructor or
    /// initializer. The grammar reads these as Java blocks, where a command
    /// expression with a literal argument (`run "x"`) is an ERROR.
    MethodBody,
    Other,
}

/// Words that start a statement other than a command expression.
const STATEMENT_KEYWORDS: &[&[u8]] = &[
    b"return",
    b"throw",
    b"assert",
    b"def",
    b"var",
    b"new",
    b"else",
    b"if",
    b"while",
    b"for",
    b"switch",
    b"case",
    b"default",
    b"try",
    b"catch",
    b"finally",
    b"do",
    b"yield",
    b"import",
    b"package",
    b"class",
    b"interface",
    b"enum",
    b"trait",
    b"static",
    b"final",
    b"private",
    b"public",
    b"protected",
    b"abstract",
    b"synchronized",
    b"void",
    b"int",
    b"long",
    b"boolean",
    b"char",
    b"byte",
    b"short",
    b"float",
    b"double",
    b"in",
    b"instanceof",
    b"as",
    b"this",
    b"super",
    b"break",
    b"continue",
    b"goto",
    b"extends",
    b"implements",
    b"null",
    b"true",
    b"false",
];

/// A command expression the grammar rejects, starting at `i`: a dotted
/// name (`run`, `result.assertTaskSkipped`), blanks, then a first argument
/// that is a dotted name (`rootProject.ext.v`) or a call (`tasks()`), or —
/// with `literals`, in a method body of a type — a string or number.
/// Returns the offset of the first blank, which becomes `+` (`run+"x"`, a
/// binary expression the grammar reads). A capitalized or keyword head is a
/// declaration or a statement, never a command.
fn command_at(src: &[u8], i: usize, literals: bool) -> Option<usize> {
    if src.get(i).is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    // A method pointer argument (`foo this.&bar`) keeps the shape the
    // plugin reads it from.
    let eol = src[i..]
        .iter()
        .position(|c| *c == b'\n')
        .map_or(src.len(), |p| i + p);
    if src[i..eol].windows(2).any(|w| w == b".&") {
        return None;
    }
    let mut j = i;
    let mut first = true;
    loop {
        if !src.get(j).copied().is_some_and(ident_start) {
            return None;
        }
        let end = src[j..]
            .iter()
            .position(|c| !ident_byte(*c))
            .map_or(src.len(), |p| j + p);
        if first
            && STATEMENT_KEYWORDS.contains(&&src[j..end])
            && src.get(end) != Some(&b'.')
        {
            return None;
        }
        first = false;
        j = end;
        if src.get(j) == Some(&b'.') {
            j += 1;
            continue;
        }
        break;
    }
    let gap = j;
    while src.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
        j += 1;
    }
    if j == gap {
        return None;
    }
    let lit = *src.get(j)?;
    if lit == b'"' || lit == b'\'' || lit.is_ascii_digit() {
        return literals.then_some(gap);
    }
    // A named argument, in a method body: `exclude module: 'x'`.
    if literals && named_key_colon(src, j).is_some() {
        return Some(gap);
    }
    // A dotted name or a call: `a.b`, `f(`, `a.b(`.
    if !ident_start(lit) {
        return None;
    }
    let end = src[j..]
        .iter()
        .position(|c| !ident_byte(*c))
        .map_or(src.len(), |p| j + p);
    if STATEMENT_KEYWORDS.contains(&&src[j..end]) {
        return None;
    }
    let follows = src.get(end).copied();
    let dotted =
        follows == Some(b'.') && src.get(end + 1).copied().is_some_and(ident_start);
    if follows == Some(b'(') {
        // `task build() {`, `foo bar(x) {`: a definition the grammar reads
        // (a return type, a name, parameters, a body), not a command. With
        // named arguments (`task copy(type: Copy) {`) it is a command.
        let close = src[end..]
            .iter()
            .position(|c| *c == b')' || *c == b'\n')
            .map(|p| end + p)?;
        if src[close] == b')' {
            let after = src[close + 1..]
                .iter()
                .position(|c| !matches!(c, b' ' | b'\t'))
                .map(|p| close + 1 + p);
            if after.is_some_and(|a| src[a] == b'{') && !src[end..close].contains(&b':') {
                return None;
            }
        }
        return Some(gap);
    }
    dotted.then_some(gap)
}

/// A named argument's key at `j` (`module: …`): the offset of its colon,
/// when a blank follows the colon (which, with the colon, becomes `==`).
fn named_key_colon(src: &[u8], j: usize) -> Option<usize> {
    if !src.get(j).copied().is_some_and(ident_start) {
        return None;
    }
    let end = src[j..]
        .iter()
        .position(|c| !ident_byte(*c))
        .map_or(src.len(), |p| j + p);
    (src.get(end) == Some(&b':')
        && src.get(end + 1).is_some_and(|c| *c == b' ' || *c == b'\t'))
    .then_some(end)
}

/// Rewrites a named argument's `key: ` at `j` (after blanks) to
/// `key==`, when there is one: `run+key== value`, a comparison.
fn rewrite_named_key(
    src: &[u8],
    mut j: usize,
    out: &mut Option<Vec<u8>>,
    set: &mut impl FnMut(&mut Option<Vec<u8>>, usize, u8),
) {
    while src.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
        j += 1;
    }
    if let Some(colon) = named_key_colon(src, j) {
        set(out, colon, b'=');
        set(out, colon + 1, b'=');
    }
}

/// Applies the rewrite that starts at `i` in code, if any, returning how
/// many bytes it covered.
fn rewrite_at(
    src: &[u8],
    i: usize,
    prev_sig: u8,
    line_start: bool,
    import_line: bool,
    out: &mut Option<Vec<u8>>,
    set: &mut impl FnMut(&mut Option<Vec<u8>>, usize, u8),
) -> Option<usize> {
    let b = src[i];
    let next = src.get(i + 1).copied();
    let after = src.get(i + 2).copied();
    // `assert a.b` (a dotted name alone, read as a type): `     !a.b`.
    if line_start && b == b'a' && src[i..].starts_with(b"assert ") {
        let end = src[i..]
            .iter()
            .position(|c| *c == b'\n')
            .map_or(src.len(), |p| i + p);
        let rest = std::str::from_utf8(&src[i + 7..end]).unwrap_or("").trim();
        let name = rest.trim_start_matches(['!', ' ']);
        if name.contains('.')
            && name.split('.').all(|seg| {
                seg.bytes().next().is_some_and(ident_start) && seg.bytes().all(ident_byte)
            })
        {
            for at in i..i + 5 {
                set(out, at, b' ');
            }
            set(out, i + 5, b'!');
            return Some(6);
        }
    }
    // A map key or named argument the grammar takes for a keyword
    // (`module:`, `open:`, `with:`, `record:`, `sealed:`, `yield:`): its
    // first letter becomes `_`.
    if matches!(prev_sig, b'[' | b',' | b'(' | b'a' | b'+')
        && (i == 0 || !(ident_byte(src[i - 1]) || src[i - 1] == b'.'))
    {
        for kw in [
            &b"module"[..],
            b"open",
            b"with",
            b"record",
            b"sealed",
            b"yield",
        ] {
            if src[i..].starts_with(kw)
                && src.get(i + kw.len()) == Some(&b':')
                && src.get(i + kw.len() + 1) != Some(&b':')
            {
                set(out, i, b'_');
                return Some(kw.len());
            }
        }
    }
    // A GPath attribute, `x.@name` / `xs*.@name`: the `@` (and the
    // spread's `*`) become spaces.
    if b == b'.' && next == Some(b'@') && after.is_some_and(ident_start) {
        if i > 0 && src[i - 1] == b'*' {
            set(out, i - 1, b' ');
        }
        set(out, i + 1, b' ');
        return Some(2);
    }
    if b == b'*' && next == Some(b'.') && after == Some(b'@') {
        set(out, i, b' ');
        set(out, i + 2, b' ');
        return Some(3);
    }
    // `a?.b`, `a*.b`: the operator byte right after a value.
    if (b == b'?' || b == b'*')
        && next == Some(b'.')
        && after.is_some_and(ident_start)
        && i > 0
        && (ident_byte(src[i - 1]) || matches!(src[i - 1], b')' | b']'))
    {
        set(out, i, b' ');
        return Some(2);
    }
    // `_ * mock.call(_)`: a Spock interaction with any count.
    if b == b'_' && line_start && !next.is_some_and(ident_byte) {
        let mut j = i + 1;
        while src.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
            j += 1;
        }
        if src.get(j) == Some(&b'*')
            && src.get(j + 1).is_some_and(|c| c.is_ascii_whitespace())
        {
            set(out, i, b'0');
            return Some(1);
        }
    }
    // `f(*args)`, `[*xs]`: a spread argument or element (not `*:`).
    if b == b'*'
        && next.is_some_and(ident_start)
        && matches!(prev_sig, b'(' | b',' | b'[')
    {
        set(out, i, b' ');
        return Some(1);
    }
    // `[:]`, the empty map: `[ ]`.
    if b == b'[' {
        let mut j = i + 1;
        while src.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
            j += 1;
        }
        if src.get(j) == Some(&b':') {
            let mut k = j + 1;
            while src.get(k).is_some_and(|c| *c == b' ' || *c == b'\t') {
                k += 1;
            }
            if src.get(k) == Some(&b']') {
                // Up to the `]`, which the caller counts as closing the `[`.
                set(out, j, b' ');
                return Some(k - i);
            }
        }
    }
    // `for (def x in xs)`: `for (    x in xs)`.
    if b == b'd'
        && src[i..].starts_with(b"def")
        && src.get(i + 3).is_some_and(|c| *c == b' ' || *c == b'\t')
        && prev_sig == b'('
        && src[..i]
            .iter()
            .rposition(|c| !c.is_ascii_whitespace())
            .is_some_and(|p| {
                src[..p].ends_with(b"for") || src[..p].trim_ascii_end().ends_with(b"for")
            })
    {
        for at in i..i + 3 {
            set(out, at, b' ');
        }
        return Some(3);
    }
    // `x as Type`.
    if b == b'a'
        && next == Some(b's')
        && !import_line
        && (i == 0 || !(ident_byte(src[i - 1]) || src[i - 1] == b'.'))
        && after.is_some_and(|c| c == b' ' || c == b'\t')
        && (ident_byte(prev_sig) || matches!(prev_sig, b')' | b']' | b'}' | b'"'))
        && let Some(end) = cast_type_end(src, i + 2)
    {
        for at in i..end {
            set(out, at, b' ');
        }
        return Some(end - i);
    }
    None
}

/// The end of a local declaration's head at `i` — modifiers (`def`,
/// `final`, `var`) and a type (a primitive, or a dotted name with type
/// arguments and array brackets) — when a name and `=` follow it.
fn declaration_head_end(src: &[u8], i: usize) -> Option<usize> {
    const MODIFIERS: &[&[u8]] = &[b"def", b"final", b"var"];
    const PRIMITIVES: &[&[u8]] = &[
        b"int", b"long", b"boolean", b"double", b"float", b"char", b"byte", b"short",
    ];
    let word = |j: usize| -> Option<(usize, &[u8])> {
        if !src.get(j).copied().is_some_and(ident_start) {
            return None;
        }
        let end = src[j..]
            .iter()
            .position(|c| !ident_byte(*c))
            .map_or(src.len(), |p| j + p);
        Some((end, &src[j..end]))
    };
    let blanks = |mut j: usize| {
        while src.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
            j += 1;
        }
        j
    };
    let mut j = i;
    let mut head_end = None;
    loop {
        let (end, w) = word(j)?;
        if MODIFIERS.contains(&w) {
            head_end = Some(end);
            j = blanks(end);
            if j == end {
                return None;
            }
            continue;
        }
        if PRIMITIVES.contains(&w) || w[0].is_ascii_uppercase() {
            head_end = Some(type_end(src, j)?);
        }
        break;
    }
    let head_end = head_end?;
    let name_at = blanks(head_end);
    if name_at == head_end {
        return None;
    }
    let (name_end, _) = word(name_at)?;
    let eq = blanks(name_end);
    (src.get(eq) == Some(&b'=') && src.get(eq + 1) != Some(&b'=')).then_some(head_end)
}

/// The end of the type an `as` names, from `from` (after the keyword):
/// blanks, then a dotted name, type arguments and array brackets. `None`
/// when what follows is no type.
fn cast_type_end(src: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    while src.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
        j += 1;
    }
    if j == from {
        return None;
    }
    type_end(src, j)
}

/// The end of the type written at `j`: a dotted name, type arguments and
/// array brackets. `None` when no type starts there.
fn type_end(src: &[u8], mut j: usize) -> Option<usize> {
    if !src.get(j).copied().is_some_and(ident_start) {
        return None;
    }
    loop {
        while src.get(j).copied().is_some_and(ident_byte) {
            j += 1;
        }
        if src.get(j) == Some(&b'.') && src.get(j + 1).copied().is_some_and(ident_start) {
            j += 1;
            continue;
        }
        break;
    }
    if src.get(j) == Some(&b'<') {
        let mut depth = 0i32;
        while let Some(&c) = src.get(j) {
            match c {
                b'<' => depth += 1,
                b'>' => {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                b'\n' | b'(' | b')' | b'{' | b'}' | b'"' | b'\'' | b'=' | b';' => {
                    return None;
                }
                _ => {}
            }
            j += 1;
        }
        if depth != 0 {
            return None;
        }
    }
    while src.get(j) == Some(&b'[') && src.get(j + 1) == Some(&b']') {
        j += 2;
    }
    Some(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw(s: &str) -> String {
        rewrite(s.as_bytes())
            .map(|v| String::from_utf8(v).unwrap())
            .unwrap_or_else(|| s.to_string())
    }

    #[test]
    fn rewrites_keep_length_and_lines() {
        for s in [
            "a?.b(c?.d)\nx*.y()\n",
            "_ * m.call(_) >> 1\nf(*args, *more)\n",
            "def s = (x as Foo).m() + g([] as Set<String>) + h(y as String[])\n",
        ] {
            let r = rw(s);
            assert_eq!(r.len(), s.len());
            assert_eq!(r.matches('\n').count(), s.matches('\n').count());
        }
    }

    #[test]
    fn each_shape_is_rewritten() {
        assert_eq!(rw("def m = [:]"), "def m = [ ]");
        assert_eq!(
            rw(
                "class A {\n  def f() { x.y() }\n  def g() { }\n  def h() { a(); }\n  def k() {\n    b() }\n}\nf() { c() }"
            ),
            "class A {\n  def f() { x.y();}\n  def g() { }\n  def h() { a(); }\n  def k() {\n    b() }\n}\nf() { c() }"
        );
        assert_eq!(rw("x = [module: 1, open: 2]"), "x = [_odule: 1, _pen: 2]");
        assert_eq!(rw("a.module\nm(with: 1)"), "a.module\nm(_ith: 1)");
        assert_eq!(
            rw("assert a.b\nassert !x.y.z\nassert a.b == 1"),
            "     ! a.b\n     ! !x.y.z\nassert a.b == 1"
        );
        assert_eq!(rw("x.@name\nxs*.@name"), "x. name\nxs . name");
        assert_eq!(
            rw("class A {\n  def f() {\n    a.exclude group: 'g', module: 'm'\n  }\n}"),
            "class A {\n  def f() {\n    a.exclude+group=='g'+ _odule=='m'\n  }\n}"
        );
        assert_eq!(
            rw(
                "xs.each { a ->\n  def d = 1\n}\nys.each { b ->\n  List<String> e = []\n  def f = 2\n}\nzs.each { c -> x == 1 }"
            ),
            "xs.each { a ->\n      d = 1\n}\nys.each { b ->\n               e = []\n  def f = 2\n}\nzs.each { c -> x == 1 }"
        );
        assert_eq!(rw("f('', 'x', '''a''')"), "f(\"\", 'x', '''a''')");
        assert_eq!(rw("f([ : ], x)"), "f([   ], x)");
        assert_eq!(rw("def p = /a.b/"), "def p = \"a.b\"");
        assert_eq!(rw("def p = ~/a\"b/"), "def p = ~'a\"b'");
        assert_eq!(rw("for (def c in cs) {}"), "for (    c in cs) {}");
        assert_eq!(rw("a?.b(c?.d)"), "a .b(c .d)");
        assert_eq!(rw("xs*.name"), "xs .name");
        assert_eq!(rw("  _ * m.call(_)"), "  0 * m.call(_)");
        assert_eq!(rw("f(*args, *more)"), "f( args,  more)");
        assert_eq!(rw("[*xs]"), "[ xs]");
        assert_eq!(rw("(x as Foo).m()"), "(x       ).m()");
        assert_eq!(
            rw("g([] as Set<String>)"),
            format!("g([]{})", " ".repeat(15))
        );
        assert_eq!(rw("h(y as String[])"), format!("h(y{})", " ".repeat(12)));
    }

    /// A command with a literal first argument, directly in a method body
    /// of a type, becomes a binary expression; its top-level commas too.
    /// Elsewhere (a script, a nested block) the grammar reads it already.
    #[test]
    fn commands_in_method_bodies_are_rewritten() {
        let s = r#"class A {
  def f() {
    run "x", g(1, 2), '''
  a, b
  '''
    result.check 'y'
    foo bar
    return 'z'
    if (c) { run 'w' }
  }
}
run 'top'
"#;
        let want = r#"class A {
  def f() {
    run+"x"+ g(1, 2)+ '''
  a, b
  '''
    result.check+'y'
    foo bar
    return 'z'
    if (c) { run 'w' }
  }
}
run 'top'
"#;
        assert_eq!(rw(s), want);
        // A dotted or call argument, anywhere but a type body; never a
        // declaration (`String name(`), a keyword or a plain name.
        let s = "android {\n  compileSdk rootProject.ext.v\n}\ntask build() {\n}\ntask foo(type: Copy) {\n}\nsign a.b, c.d\nfoo bar\nString name(int x) { }\nreturn a.b\nclass B {\n  void m(int x) { }\n}\n";
        let want = "android {\n  compileSdk+rootProject.ext.v\n}\ntask build() {\n}\ntask+foo(type: Copy) {\n}\nsign+a.b+ c.d\nfoo bar\nString name(int x) { }\nreturn a.b\nclass B {\n  void m(int x) { }\n}\n";
        assert_eq!(rw(s), want);
        // `Foo.class` and a `class:` key open no type body.
        let s = "def f() {\n  def k = Foo.class\n  m(class: 1) {\n    run 'x'\n  }\n}\n";
        assert_eq!(rw(s), s);
    }

    #[test]
    fn strings_comments_imports_and_near_misses_are_left_alone() {
        for s in [
            "import a.b.C as D\n",
            "def s = 'a?.b as Foo'\n",
            "def s = \"x ${a?.b} y\"\n",
            "def s = '''\n a?.b\n'''\n",
            "// a?.b as Foo\n",
            "/* _ * x */\n",
            "def r = /a?.b/\n",
            "m(as: 1)\n",
            "x.as(Foo)\n",
            "a ? .5 : 1\n",
            "a * b\n",
            "f(a * b)\n",
            "[a: 1, *: m]\n",
            "x = y / z?.w\n",
        ] {
            // A division is code; a slashy string keeps its content and
            // only trades its delimiters for quotes.
            let expect = match s {
                "x = y / z?.w\n" => "x = y / z .w\n".to_string(),
                "def r = /a?.b/\n" => "def r = \"a?.b\"\n".to_string(),
                _ => s.to_string(),
            };
            assert_eq!(rw(s), expect, "{s}");
        }
    }
}
