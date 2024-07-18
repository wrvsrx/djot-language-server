#include "tree_sitter/alloc.h"
#include "tree_sitter/array.h"
#include "tree_sitter/parser.h"
#include <stdio.h>

// #define TREE_SITTER_DEBUG

#ifdef TREE_SITTER_DEBUG
#include <assert.h>
#endif

// The different tokens the external scanner support
// See `externals` in `grammar.js` for a description of most of them.
enum TokenType {
  BLOCK_LIKE_START,
  BLOCK_LIKE_END,
  SOFTBREAK,
  IGNORED,
};

enum BlockLikeType {
  PARAGRAPH,
  BLANKLINE,
};

struct Empty {};

union BlockLikeMetadata {
  struct Empty paragraph;
  struct Empty blankline;
};

struct BlockLike {
  enum BlockLikeType type;
  union BlockLikeMetadata metadata;
};
typedef Array(struct BlockLike) BlockLikeStack;

enum LineParsingState {
  START_PARSING_IGNORED,
  JUST_PARSED_IGNORED,
  OTHERWISE,
};

struct ScannerState {
  // have we parse the line start
  // use this flag to avoid stuck at empty line
  // this flag is reset every time we consume '\n'
  enum LineParsingState line_parsing_state;
  // store all open blocks
  BlockLikeStack block_like_stack;
};

void init(struct ScannerState *s) {
  array_init(&(s->block_like_stack));
  s->line_parsing_state = START_PARSING_IGNORED;
}
void *tree_sitter_djot_external_scanner_create(void) {
  struct ScannerState *s = ts_malloc(sizeof(struct ScannerState));
  init(s);
  return s;
}

void tree_sitter_djot_external_scanner_destroy(void *payload) {
  struct ScannerState *s = payload;
  ts_free(s);
}

#define SAVE_TO_BUFFER(buffer, size, value)                                    \
  *(__typeof__(value) *)(buffer + size) = value;                               \
  size += sizeof(__typeof__(value));

#define LOAD_FROM_BUFFER(buffer, size, value)                                  \
  value = *(__typeof__(value) *)(buffer + size);                               \
  size += sizeof(__typeof__(value));

unsigned tree_sitter_djot_external_scanner_serialize(void *payload,
                                                     char *buffer) {
  struct ScannerState *s = payload;
  unsigned size = 0;
  SAVE_TO_BUFFER(buffer, size, s->line_parsing_state);
  SAVE_TO_BUFFER(buffer, size, s->block_like_stack.size);
  for (__typeof__(s->block_like_stack.size) i = 0; i < s->block_like_stack.size;
       ++i) {
    SAVE_TO_BUFFER(buffer, size, s->block_like_stack.contents[i]);
  }
  return size;
}

void tree_sitter_djot_external_scanner_deserialize(void *payload,
                                                   const char *buffer,
                                                   unsigned length) {
  struct ScannerState *s = payload;
  init(s);

  if (length == 0) {
    return;
  }

  unsigned size = 0;
  LOAD_FROM_BUFFER(buffer, size, s->line_parsing_state);
  __typeof__(s->block_like_stack.size) count;
  LOAD_FROM_BUFFER(buffer, size, count);

  array_grow_by(&(s->block_like_stack), count);
  for (__typeof__(count) i = 0; i < count; ++i) {
    LOAD_FROM_BUFFER(buffer, size, s->block_like_stack.contents[i]);
  }
  assert(length == size);
}

void consume_whitespace_at_start(TSLexer *lexer) {
  assert(lexer->get_column(lexer) == 0);
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
    lexer->advance(lexer, true);
  }
}

void push_block_like(struct ScannerState *s, struct BlockLike b) {
#ifdef TREE_SITTER_DEBUG
  if (b.type == PARAGRAPH) {
    printf("---push paragraph\n");
  } else if (b.type == BLANKLINE) {
    printf("---push blankline\n");
  }
#endif
  array_push(&(s->block_like_stack), b);
}

void pop_block_like(struct ScannerState *s) {
  struct BlockLike t = *array_back(&(s->block_like_stack));
#ifdef TREE_SITTER_DEBUG
  if (t.type == PARAGRAPH) {
    printf("---pop paragraph\n");
  } else if (t.type == BLANKLINE) {
    printf("---pop blankline\n");
  }
#endif
  array_pop(&(s->block_like_stack));
}

static void accpet_block_like_end(struct ScannerState *s, TSLexer *lexer,
                                  const bool *valid_symbols) {
  assert(valid_symbols[BLOCK_LIKE_END]);
  lexer->result_symbol = BLOCK_LIKE_END;
  pop_block_like(s);
}

static void accpet_softbreak(struct ScannerState *s, TSLexer *lexer,
                             const bool *valid_symbols) {
#ifdef TREE_SITTER_DEBUG
  printf("--- accept softbreak\n");
#endif
  assert(valid_symbols[SOFTBREAK]);
  lexer->result_symbol = SOFTBREAK;
}

static bool parse_eol(struct ScannerState *s, TSLexer *lexer,
                      const bool *valid_symbols) {
  // if it's eol
  // we must accpet that since we always parse eol manually
  lexer->advance(lexer, false);
  lexer->mark_end(lexer);
  s->line_parsing_state = START_PARSING_IGNORED;
#ifdef TREE_SITTER_DEBUG
  printf("--- state from OTHERWISE to START_PARSING_IGNORED\n");
#endif
  assert(s->block_like_stack.size > 0);
  struct BlockLike t = *array_back(&(s->block_like_stack));
  if (t.type == BLANKLINE) {
    accpet_block_like_end(s, lexer, valid_symbols);
  } else if (t.type == PARAGRAPH) {
    if (lexer->eof(lexer)) {
      accpet_block_like_end(s, lexer, valid_symbols);
    } else {
      consume_whitespace_at_start(lexer);
      bool is_newline = lexer->lookahead == '\n';
      if (is_newline) {
        accpet_block_like_end(s, lexer, valid_symbols);
      } else {
        accpet_softbreak(s, lexer, valid_symbols);
      }
    }
  } else {
    assert(false);
  }
  return true;
}

bool tree_sitter_djot_external_scanner_scan(void *payload, TSLexer *lexer,
                                            const bool *valid_symbols) {
  struct ScannerState *s = payload;

  if (lexer->eof(lexer)) {
    // it might not true when we have other kind of blocks
    // it's only true now
    assert(s->block_like_stack.size == 0);
    return false;
  }

  // when parsing, we have three possible case
  // 1. at the start of line, and we haven't parsed anything
  // 2. we have comsumed all leading whitespaces (zero or more) but havn't do
  // anything else
  // 3. we have done something
  if (s->line_parsing_state == START_PARSING_IGNORED) {
    s->line_parsing_state = JUST_PARSED_IGNORED;
#ifdef TREE_SITTER_DEBUG
    printf("--- state from START_PARSING_IGNORED to JUST_PARSED_IGNORED\n");
#endif
    assert(lexer->get_column(lexer) == 0);
    // if it's start
    // jump over leading spaces, treat them as ignored
    consume_whitespace_at_start(lexer);
    lexer->mark_end(lexer);
    assert(valid_symbols[IGNORED]);
    lexer->result_symbol = IGNORED;
    return true;
  } else if (s->line_parsing_state == JUST_PARSED_IGNORED) {
    s->line_parsing_state = OTHERWISE;
#ifdef TREE_SITTER_DEBUG
    printf("--- state from JUST_PARSED_IGNORED to OTHERWISE\n");
#endif
    if (s->block_like_stack.size == 0) {
      // if no block is open, search for newline or paragraph
      if (lexer->lookahead == '\n') {
        struct BlockLike b = {.type = BLANKLINE, .metadata = {.blankline = {}}};
        push_block_like(s, b);
      } else {
        struct BlockLike b = {.type = PARAGRAPH, .metadata = {.paragraph = {}}};
        push_block_like(s, b);
      }
      assert(valid_symbols[BLOCK_LIKE_START]);
      lexer->result_symbol = BLOCK_LIKE_START;
    } else {
      struct BlockLike t = *array_back(&(s->block_like_stack));
      if (t.type == PARAGRAPH) {
        // if current block is paragraph, continue parsing
      } else if (t.type == BLANKLINE) {
        // if current block is blankline, it's impossible
        assert(false);
      }
      assert(valid_symbols[IGNORED]);
      lexer->result_symbol = IGNORED;
    }
    return true;
  } else if (s->line_parsing_state == OTHERWISE) {
#ifdef TREE_SITTER_DEBUG
    printf("--- counter %c\n", lexer->lookahead);
#endif
    if (lexer->lookahead == '\n') {
      // if it isn't start or start has been parsed
      assert(parse_eol(s, lexer, valid_symbols));
      return true;
    }
  } else {
    assert(false);
  }

  return false;
}
