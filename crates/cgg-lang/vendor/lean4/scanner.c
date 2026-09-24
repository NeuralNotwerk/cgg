/**
 * External scanner for Lean 4 tree-sitter grammar.
 * Handles layout/indentation-sensitive parsing.
 *
 * Lean uses indentation to delimit blocks after `do`, `where`, `:=`, `=>`.
 * The scanner maintains a stack of indent levels and emits:
 *   LAYOUT_START     — pushed when grammar enters a layout context
 *   LAYOUT_SEMICOLON — same-indent newline within a layout block
 *   LAYOUT_END       — indent decreased below current layout level
 *   MATCH_BODY_START — like LAYOUT_START but dedicated to match arm bodies
 *
 * MATCH_BODY_START is a separate token so match_arm can open a layout
 * block without creating grammar conflicts with _do_element.  It pushes
 * the same indent stack as LAYOUT_START and is closed by LAYOUT_END.
 *
 * Key design: a `queued_indent` field persists across scanner calls so that
 * a single newline can produce multiple LAYOUT_END tokens (one per popped
 * level) followed by a LAYOUT_SEMICOLON, across successive scanner invocations.
 *
 * Special case: `|` at the start of a line does NOT get a LAYOUT_SEMICOLON.
 * This matches Lean 4's parser where match arms are delimited by `|` tokens
 * rather than by the indentation-based semicolon mechanism.
 */

#include "tree_sitter/parser.h"
#include "tree_sitter/alloc.h"
#include <string.h>

enum TokenType {
  LAYOUT_START,
  LAYOUT_SEMICOLON,
  LAYOUT_END,
  MATCH_BODY_START,
  SYNTAX_QUOTATION_BODY,  // content inside `` `( ... ) `` up to matching `)`
  BRACE_FIELD_SEP,        // newline-as-separator inside `{ … }` struct instance
};

#define MAX_DEPTH 64
#define NO_QUEUED UINT32_MAX

typedef struct {
  uint32_t indents[MAX_DEPTH];
  uint8_t  depth;
  uint32_t queued_indent; // indent of next non-blank line, or NO_QUEUED
} Scanner;

/* ── lifecycle ─────────────────────────────────────────────────── */

void *tree_sitter_lean_external_scanner_create(void) {
  Scanner *s = ts_calloc(1, sizeof(Scanner));
  s->depth = 0;
  s->queued_indent = NO_QUEUED;
  return s;
}

void tree_sitter_lean_external_scanner_destroy(void *payload) {
  ts_free(payload);
}

unsigned tree_sitter_lean_external_scanner_serialize(void *payload, char *buffer) {
  Scanner *s = (Scanner *)payload;
  unsigned pos = 0;
  memcpy(buffer + pos, &s->depth, sizeof(s->depth));       pos += sizeof(s->depth);
  memcpy(buffer + pos, &s->queued_indent, sizeof(s->queued_indent)); pos += sizeof(s->queued_indent);
  unsigned indent_bytes = s->depth * sizeof(uint32_t);
  memcpy(buffer + pos, s->indents, indent_bytes);           pos += indent_bytes;
  return pos;
}

void tree_sitter_lean_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  Scanner *s = (Scanner *)payload;
  if (length == 0) {
    s->depth = 0;
    s->queued_indent = NO_QUEUED;
    return;
  }
  unsigned pos = 0;
  memcpy(&s->depth, buffer + pos, sizeof(s->depth));        pos += sizeof(s->depth);
  memcpy(&s->queued_indent, buffer + pos, sizeof(s->queued_indent)); pos += sizeof(s->queued_indent);
  if (s->depth > MAX_DEPTH) { s->depth = 0; s->queued_indent = NO_QUEUED; return; }
  memcpy(s->indents, buffer + pos, s->depth * sizeof(uint32_t));
}

/* ── helpers ───────────────────────────────────────────────────── */

static inline uint32_t top_indent(Scanner *s) {
  return s->depth > 0 ? s->indents[s->depth - 1] : 0;
}

/* The layout column just below the top — what we'd see after one pop. */
static inline uint32_t penultimate_indent(Scanner *s) {
  return s->depth > 1 ? s->indents[s->depth - 2] : 0;
}

static inline void push(Scanner *s, uint32_t indent) {
  if (s->depth < MAX_DEPTH) {
    s->indents[s->depth++] = indent;
  }
}

static inline void pop(Scanner *s) {
  if (s->depth > 0) s->depth--;
}

static void skip_spaces(TSLexer *lexer) {
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t')
    lexer->advance(lexer, true);
}

static bool is_nl(int32_t c) { return c == '\n' || c == '\r'; }

/**
 * Skip newlines + leading whitespace, return column of first non-blank char.
 * If EOF is reached, return 0 (dedent everything).
 * After this call, lexer->lookahead is the first non-blank character.
 */
static uint32_t measure_indent(TSLexer *lexer) {
  while (is_nl(lexer->lookahead))
    lexer->advance(lexer, true);
  skip_spaces(lexer);
  if (lexer->eof(lexer)) return 0;
  return lexer->get_column(lexer);
}

/**
 * In Lean 4, match arms are delimited by `|` without semicolons.
 * A `|` at the start of a line should also not trigger LAYOUT_END,
 * because match arms may appear at a column lower than the enclosing
 * binding's layout (e.g. `def f := match x with | ...` puts arms
 * below the `match` column).
 */
static bool starts_with_pipe(TSLexer *lexer) {
  return lexer->lookahead == '|';
}

static bool should_suppress_semicolon(TSLexer *lexer) {
  return starts_with_pipe(lexer);
}

/**
 * Push indent for a layout-start-like token (shared by LAYOUT_START
 * and MATCH_BODY_START).
 */
static void push_layout_indent(Scanner *s, TSLexer *lexer) {
  skip_spaces(lexer);
  if (is_nl(lexer->lookahead)) {
    uint32_t indent = measure_indent(lexer);
    push(s, indent);
  } else {
    push(s, lexer->get_column(lexer));
  }
  s->queued_indent = NO_QUEUED;
}

/* ── main scan ─────────────────────────────────────────────────── */

bool tree_sitter_lean_external_scanner_scan(
    void *payload, TSLexer *lexer, const bool *valid_symbols) {

  Scanner *s = (Scanner *)payload;

  /* 0. Error recovery: if all layout symbols are valid simultaneously
        the parser is in error recovery mode — don't interfere. */
  if (valid_symbols[LAYOUT_START] &&
      valid_symbols[LAYOUT_SEMICOLON] &&
      valid_symbols[LAYOUT_END]) {
    return false;
  }

  /* 1y. BRACE_FIELD_SEP — inside `{ … }`, a newline acts as a field
         separator. Emit only when the parser asks for it (between two
         field assignments). The structure-instance rule places this in the
         separator position so a missing comma is acceptable when newlines
         intervene. */
  if (valid_symbols[BRACE_FIELD_SEP]) {
    skip_spaces(lexer);
    if (is_nl(lexer->lookahead)) {
      lexer->mark_end(lexer);
      while (is_nl(lexer->lookahead)) {
        lexer->advance(lexer, true);
        skip_spaces(lexer);
      }
      // Don't consume up to `}` — let the parser see it as end-of-body.
      if (lexer->lookahead != '}' && !lexer->eof(lexer)) {
        lexer->result_symbol = BRACE_FIELD_SEP;
        return true;
      }
    }
  }

  /* 1z. SYNTAX_QUOTATION_BODY — the parser has just consumed `` `( `` and
         expects the body of a syntax quotation. Greedily read until the
         matching `)`, tracking nested parens, brackets, and braces.
         Stops just before the closing `)` so the grammar can consume it. */
  if (valid_symbols[SYNTAX_QUOTATION_BODY]) {
    int paren_depth = 0;
    int bracket_depth = 0;
    int brace_depth = 0;
    bool consumed_any = false;
    while (!lexer->eof(lexer)) {
      int32_t c = lexer->lookahead;
      if (c == ')' && paren_depth == 0 && bracket_depth == 0 && brace_depth == 0) {
        break;
      }
      if (c == '(') paren_depth++;
      else if (c == ')') { if (paren_depth > 0) paren_depth--; }
      else if (c == '[') bracket_depth++;
      else if (c == ']') { if (bracket_depth > 0) bracket_depth--; }
      else if (c == '{') brace_depth++;
      else if (c == '}') { if (brace_depth > 0) brace_depth--; }
      lexer->advance(lexer, false);
      consumed_any = true;
    }
    if (consumed_any) {
      lexer->mark_end(lexer);
      lexer->result_symbol = SYNTAX_QUOTATION_BODY;
      return true;
    }
    return false;
  }

  /* 1a. LAYOUT_START — grammar just saw `do`, `where`, `:=`, `=>`
         and wants to open a new layout block. */
  if (valid_symbols[LAYOUT_START]) {
    push_layout_indent(s, lexer);
    lexer->result_symbol = LAYOUT_START;
    return true;
  }

  /* 1b. MATCH_BODY_START — grammar just saw `=>` in a match arm.
         Pushes indent identically to LAYOUT_START, but uses a
         distinct token so the parser can distinguish match arm
         context from general layout (avoiding conflicts with
         _do_element).  Closed by the same LAYOUT_END mechanism. */
  if (valid_symbols[MATCH_BODY_START]) {
    push_layout_indent(s, lexer);
    lexer->result_symbol = MATCH_BODY_START;
    return true;
  }

  /* 2. Process queued indent from a previous newline.
        Each call pops at most one layout level (LAYOUT_END) or emits
        LAYOUT_SEMICOLON, then returns so tree-sitter can re-enter. */
  if (s->queued_indent != NO_QUEUED && s->depth > 0) {
    uint32_t qi = s->queued_indent;
    uint32_t ci = top_indent(s);

    if (qi < ci && valid_symbols[LAYOUT_END]) {
      // Top-level match arms below the body's indent (`def f := match X with
      // | ...`): when `:=` is the only layout open and the next token is `|`,
      // the arms belong to the match. Closing the layout would terminate the
      // match prematurely. Deeper stacks pop normally so nested matches still
      // work — there an inner match needs LAYOUT_END to fire so the outer
      // match's arm at a lower column can fire.
      if (s->depth == 1) {
        lexer->mark_end(lexer);
        skip_spaces(lexer);
        while (is_nl(lexer->lookahead)) {
          lexer->advance(lexer, true);
          skip_spaces(lexer);
        }
        if (starts_with_pipe(lexer)) {
          s->queued_indent = NO_QUEUED;
          return false;
        }
      }
      pop(s);
      lexer->result_symbol = LAYOUT_END;
      return true;
    }
    if (qi == ci && valid_symbols[LAYOUT_SEMICOLON]) {
      // Peek ahead past newlines+whitespace to see the actual next token.
      // Don't emit semicolon before `|` — match arms are delimited
      // by `|` tokens, not by layout semicolons.
      skip_spaces(lexer);
      while (is_nl(lexer->lookahead)) {
        lexer->advance(lexer, true);
        skip_spaces(lexer);
      }
      if (should_suppress_semicolon(lexer)) {
        s->queued_indent = NO_QUEUED;
        return false;
      }
      s->queued_indent = NO_QUEUED;
      lexer->result_symbol = LAYOUT_SEMICOLON;
      return true;
    }
    s->queued_indent = NO_QUEUED;
  }

  /* 3. Skip horizontal whitespace before inspecting the character. */
  skip_spaces(lexer);

  /* 4. Newline — measure indent of next line and start processing. */
  if (is_nl(lexer->lookahead) && s->depth > 0) {
    lexer->mark_end(lexer);
    uint32_t next = measure_indent(lexer);
    uint32_t ci   = top_indent(s);

    if (next < ci && valid_symbols[LAYOUT_END]) {
      // Mirror of the depth==1 / `|` guard in step 2.
      if (s->depth == 1 && starts_with_pipe(lexer)) {
        return false;
      }
      pop(s);
      s->queued_indent = next;
      lexer->result_symbol = LAYOUT_END;
      return true;
    }
    if (next == ci && valid_symbols[LAYOUT_SEMICOLON]) {
      // Suppress semicolon before `|`
      if (should_suppress_semicolon(lexer)) {
        s->queued_indent = NO_QUEUED;
        return false;
      }
      s->queued_indent = NO_QUEUED;
      lexer->result_symbol = LAYOUT_SEMICOLON;
      return true;
    }
    return false;
  }

  /* 5. EOF while in layout — emit LAYOUT_END to close all open blocks. */
  if (lexer->eof(lexer) && s->depth > 0 && valid_symbols[LAYOUT_END]) {
    pop(s);
    lexer->result_symbol = LAYOUT_END;
    return true;
  }

  /* 6. Closing bracket/paren/brace — force-close one layout level. */
  if (valid_symbols[LAYOUT_END] && s->depth > 0) {
    int32_t c = lexer->lookahead;
    if (c == ')' || c == ']' || c == '}') {
      pop(s);
      lexer->result_symbol = LAYOUT_END;
      return true;
    }
  }

  return false;
}
