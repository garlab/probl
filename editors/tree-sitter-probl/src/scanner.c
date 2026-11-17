#include "tree_sitter/parser.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

enum Token {
  NEWLINE, SOFT_NEWLINE, END,
  LPAREN, RPAREN, LBRACKET, RBRACKET, LBRACE, RBRACE,
  STRING_START, STRING_END,
  ERROR_SENTINEL,
};

// The compiler limits nesting to 200. Keep scanner state bounded and wholly
// serializable, including for malformed input and incremental reparses.
#define MAX_DEPTH 256
_Static_assert(MAX_DEPTH + 7 <= TREE_SITTER_SERIALIZATION_BUFFER_SIZE, "scanner state must fit");
typedef struct {
  uint16_t depth;
  uint32_t following_newlines;
  bool before_dot;
  uint8_t brackets[MAX_DEPTH];
} Scanner;

void *tree_sitter_probl_external_scanner_create(void) { return calloc(1, sizeof(Scanner)); }
void tree_sitter_probl_external_scanner_destroy(void *payload) { free(payload); }
unsigned tree_sitter_probl_external_scanner_serialize(void *payload, char *buffer) {
  const Scanner *s = payload;
  buffer[0] = (char)(s->depth & 255);
  buffer[1] = (char)(s->depth >> 8);
  for (unsigned i = 0; i < 4; i++) buffer[2 + i] = (char)(s->following_newlines >> (8 * i));
  buffer[6] = s->before_dot;
  memcpy(buffer + 7, s->brackets, s->depth);
  return s->depth + 7;
}
void tree_sitter_probl_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  Scanner *s = payload;
  memset(s, 0, sizeof(*s));
  if (length < 7) return;
  unsigned depth = (uint8_t)buffer[0] | ((unsigned)(uint8_t)buffer[1] << 8);
  if (depth > MAX_DEPTH || length != depth + 7) return;
  s->depth = (uint16_t)depth;
  for (unsigned i = 0; i < 4; i++) s->following_newlines |= (uint32_t)(uint8_t)buffer[2 + i] << (8 * i);
  s->before_dot = buffer[6] != 0;
  memcpy(s->brackets, buffer + 7, depth);
}

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }
static void spaces(TSLexer *lexer) {
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t' || lexer->lookahead == '\r')
    lexer->advance(lexer, true);
}

bool tree_sitter_probl_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid) {
  Scanner *s = payload;
  if (valid[ERROR_SENTINEL]) return false;
  // Whitespace at the end of literal text belongs to string_content.
  if (valid[STRING_END] && s->depth && s->brackets[s->depth - 1] == '"') {
    if (lexer->lookahead != '"') return false;
    s->depth--;
    advance(lexer);
    lexer->result_symbol = STRING_END;
    return true;
  }
  spaces(lexer);
  if (valid[STRING_START] && lexer->lookahead == '"') {
    if (s->depth == MAX_DEPTH) return false;
    s->brackets[s->depth++] = '"';
    advance(lexer);
    lexer->result_symbol = STRING_START;
    return true;
  }
  if (valid[END] && (lexer->lookahead == '}' || lexer->eof(lexer))) {
    lexer->result_symbol = END;
    return true;
  }

  unsigned symbol;
  switch (lexer->lookahead) {
    case '(': symbol = LPAREN; break;
    case ')': symbol = RPAREN; break;
    case '[': symbol = LBRACKET; break;
    case ']': symbol = RBRACKET; break;
    case '{': symbol = LBRACE; break;
    case '}': symbol = RBRACE; break;
    default: symbol = ERROR_SENTINEL; break;
  }
  if (symbol != ERROR_SENTINEL && valid[symbol]) {
    if (symbol == LPAREN || symbol == LBRACKET || symbol == LBRACE) {
      if (s->depth == MAX_DEPTH) return false;
      s->brackets[s->depth++] = (uint8_t)lexer->lookahead;
    } else if (s->depth) {
      s->depth--;
    }
    advance(lexer);
    lexer->result_symbol = symbol;
    return true;
  }

  if (lexer->lookahead != '\n') return false;
  // Interpolation may nest expressions and strings, but a literal newline is
  // forbidden anywhere inside the outer string, just as in the compiler.
  for (unsigned i = 0; i < s->depth; i++)
    if (s->brackets[i] == '"') return false;
  advance(lexer);
  lexer->mark_end(lexer);
  // Look through blank/comment lines for a method-chain dot. Marking the end
  // before lookahead keeps comments in the tree and available to queries.
  // Cache the run length so a long run costs O(n), not O(n squared). Tree-sitter
  // tracks the lookahead extent and invalidates this state when that text edits.
  uint32_t following = s->following_newlines;
  bool dot = s->before_dot;
  if (following) {
    following--;
  } else {
    for (;;) {
      spaces(lexer);
      if (lexer->lookahead == '#') {
        while (lexer->lookahead != '\n' && !lexer->eof(lexer)) advance(lexer);
      }
      if (lexer->lookahead != '\n') break;
      if (following < UINT32_MAX) following++;
      advance(lexer);
    }
    dot = false;
    if (lexer->lookahead == '.') {
      advance(lexer);
      dot = lexer->lookahead != '.';
    }
  }
  bool parens = s->depth && (s->brackets[s->depth - 1] == '(' || s->brackets[s->depth - 1] == '[');
  if (valid[SOFT_NEWLINE] && (dot || parens)) {
    lexer->result_symbol = SOFT_NEWLINE;
  } else if (valid[NEWLINE]) {
    lexer->result_symbol = NEWLINE;
  } else return false;
  s->following_newlines = following;
  s->before_dot = dot;
  return true;
}
