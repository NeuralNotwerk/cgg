
#include "tree_sitter/parser.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

enum TokenType {
  INDENT,
  DEDENT,
  INDENT_ERROR,
  SECTION_END,
};

typedef struct {
  uint16_t *levels;
  uint16_t length;
  uint16_t capacity;
  uint16_t pending_dedents;
  bool pending_indent_error;
} Scanner;

static bool reserve_levels(Scanner *scanner, uint16_t capacity) {
  uint16_t *levels;

  if (capacity <= scanner->capacity) {
    return true;
  }

  levels =
      realloc(scanner->levels, (size_t)capacity * sizeof(*scanner->levels));

  if (levels == NULL) {
    return false;
  }

  scanner->levels = levels;
  scanner->capacity = capacity;
  return true;
}

static bool push_level(Scanner *scanner, uint16_t level) {
  uint16_t capacity;

  if (scanner->length == scanner->capacity) {
    capacity = scanner->capacity == 0 ? 8 : (uint16_t)(scanner->capacity * 2);

    if (!reserve_levels(scanner, capacity)) {
      return false;
    }
  }

  scanner->levels[scanner->length] = level;
  scanner->length += 1;
  return true;
}

static uint16_t current_level(const Scanner *scanner) {
  if (scanner->length == 0) {
    return 0;
  }

  return scanner->levels[scanner->length - 1];
}

static uint16_t indentation(TSLexer *lexer) {
  uint32_t width = 0;

  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
    if (lexer->lookahead == '\t') {
      width += 8 - (width % 8);
    } else {
      width += 1;
    }

    lexer->advance(lexer, true);
  }

  if (width > UINT16_MAX) {
    return UINT16_MAX;
  }

  return (uint16_t)width;
}

static void advance_to_line_end(TSLexer *lexer) {
  while (!lexer->eof(lexer) && lexer->lookahead != '\r' &&
         lexer->lookahead != '\n') {
    lexer->advance(lexer, false);
  }
}

static bool consume_line_break(TSLexer *lexer) {
  if (lexer->lookahead == '\r') {
    lexer->advance(lexer, false);

    if (lexer->lookahead == '\n') {
      lexer->advance(lexer, false);
    }

    return true;
  }

  if (lexer->lookahead != '\n') {
    return false;
  }

  lexer->advance(lexer, false);
  return true;
}

static bool consume_section_keyword(TSLexer *lexer) {
  static const char keyword[] = "section";
  size_t index;

  for (index = 0; index < sizeof(keyword) - 1; index += 1) {
    if (lexer->lookahead != keyword[index]) {
      return false;
    }

    lexer->advance(lexer, false);
  }

  return lexer->lookahead == ':';
}

static bool section_group_starts(TSLexer *lexer, uint16_t expected_width) {
  uint16_t width = expected_width;

  while (lexer->lookahead == '#' && width == expected_width) {
    lexer->advance(lexer, false);

    if (lexer->lookahead == ':') {
      return false;
    }

    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
      lexer->advance(lexer, false);
    }

    if (consume_section_keyword(lexer)) {
      return true;
    }

    advance_to_line_end(lexer);

    if (!consume_line_break(lexer)) {
      return false;
    }

    width = indentation(lexer);
  }

  return false;
}

static bool scan_section_end(const Scanner *scanner, TSLexer *lexer) {
  uint32_t column;
  uint16_t width;
  uint16_t expected_width;

  lexer->mark_end(lexer);

  if (lexer->eof(lexer)) {
    return true;
  }

  column = lexer->get_column(lexer);

  if (column == 0) {
    width = indentation(lexer);
  } else if (column > UINT16_MAX) {
    width = UINT16_MAX;
  } else {
    width = (uint16_t)column;
  }

  if (lexer->eof(lexer)) {
    return true;
  }

  if (lexer->lookahead == '\r' || lexer->lookahead == '\n') {
    return false;
  }

  expected_width = current_level(scanner);

  if (width < expected_width) {
    return true;
  }

  if (width != expected_width) {
    return false;
  }

  return section_group_starts(lexer, expected_width);
}

void *tree_sitter_kivy_external_scanner_create(void) {
  Scanner *scanner = calloc(1, sizeof(*scanner));

  if (scanner == NULL) {
    return NULL;
  }

  if (!push_level(scanner, 0)) {
    free(scanner);
    return NULL;
  }

  return scanner;
}

void tree_sitter_kivy_external_scanner_destroy(void *payload) {
  Scanner *scanner = payload;

  if (scanner == NULL) {
    return;
  }

  free(scanner->levels);
  free(scanner);
}

unsigned tree_sitter_kivy_external_scanner_serialize(void *payload,
                                                     char *buffer) {
  Scanner *scanner = payload;
  unsigned size = 0;
  uint16_t index;

  if (scanner == NULL || scanner->length > UINT8_MAX) {
    return 0;
  }

  buffer[size++] = (char)scanner->length;
  buffer[size++] = (char)scanner->pending_dedents;
  buffer[size++] = scanner->pending_indent_error ? 1 : 0;

  for (index = 0; index < scanner->length; index += 1) {
    if (size + 2 > TREE_SITTER_SERIALIZATION_BUFFER_SIZE) {
      return 0;
    }

    buffer[size++] = (char)(scanner->levels[index] & 0xff);
    buffer[size++] = (char)(scanner->levels[index] >> 8);
  }

  return size;
}

void tree_sitter_kivy_external_scanner_deserialize(void *payload,
                                                   const char *buffer,
                                                   unsigned length) {
  Scanner *scanner = payload;
  uint16_t level_count;
  uint16_t index;
  unsigned offset = 0;

  if (scanner == NULL) {
    return;
  }

  scanner->length = 0;
  scanner->pending_dedents = 0;
  scanner->pending_indent_error = false;

  if (length < 3) {
    push_level(scanner, 0);
    return;
  }

  level_count = (uint8_t)buffer[offset++];
  scanner->pending_dedents = (uint8_t)buffer[offset++];
  scanner->pending_indent_error = buffer[offset++] != 0;

  if (level_count == 0 || length < 3 + (unsigned)level_count * 2 ||
      !reserve_levels(scanner, level_count)) {
    push_level(scanner, 0);
    scanner->pending_dedents = 0;
    scanner->pending_indent_error = false;
    return;
  }

  for (index = 0; index < level_count; index += 1) {
    uint16_t low = (uint8_t)buffer[offset++];
    uint16_t high = (uint8_t)buffer[offset++];

    scanner->levels[index] = (uint16_t)(low | (high << 8));
  }

  scanner->length = level_count;
}

bool tree_sitter_kivy_external_scanner_scan(void *payload, TSLexer *lexer,
                                            const bool *valid_symbols) {
  Scanner *scanner = payload;
  uint16_t width;
  uint16_t previous;
  uint16_t dedent_count = 0;

  if (scanner == NULL) {
    return false;
  }

  if (valid_symbols[SECTION_END] && scan_section_end(scanner, lexer)) {
    lexer->result_symbol = SECTION_END;
    return true;
  }

  if (scanner->pending_dedents > 0 && valid_symbols[DEDENT]) {
    scanner->pending_dedents -= 1;
    lexer->result_symbol = DEDENT;
    return true;
  }

  if (scanner->pending_indent_error && valid_symbols[INDENT_ERROR]) {
    indentation(lexer);
    lexer->mark_end(lexer);
    scanner->pending_indent_error = false;
    lexer->result_symbol = INDENT_ERROR;
    return true;
  }

  if (!valid_symbols[INDENT] && !valid_symbols[DEDENT] &&
      !valid_symbols[INDENT_ERROR] && !valid_symbols[SECTION_END]) {
    return false;
  }

  if (lexer->eof(lexer)) {
    if (scanner->length <= 1 || !valid_symbols[DEDENT]) {
      return false;
    }

    scanner->length -= 1;
    lexer->result_symbol = DEDENT;
    return true;
  }

  if (lexer->get_column(lexer) != 0) {
    return false;
  }

  lexer->mark_end(lexer);
  width = indentation(lexer);

  if (lexer->lookahead == '\r' || lexer->lookahead == '\n') {
    return false;
  }

  previous = current_level(scanner);

  if (width > previous) {
    if (valid_symbols[INDENT]) {
      if (!push_level(scanner, width)) {
        return false;
      }

      lexer->mark_end(lexer);
      lexer->result_symbol = INDENT;
      return true;
    }

    if (valid_symbols[INDENT_ERROR]) {
      lexer->mark_end(lexer);
      lexer->result_symbol = INDENT_ERROR;
      return true;
    }

    return false;
  }

  if (width >= previous || !valid_symbols[DEDENT]) {
    return false;
  }

  while (scanner->length > 1 && width < current_level(scanner)) {
    scanner->length -= 1;
    dedent_count += 1;
  }

  if (dedent_count == 0) {
    return false;
  }

  scanner->pending_dedents = (uint16_t)(dedent_count - 1);

  if (width != current_level(scanner)) {
    scanner->pending_indent_error = true;
  }

  lexer->result_symbol = DEDENT;
  return true;
}
