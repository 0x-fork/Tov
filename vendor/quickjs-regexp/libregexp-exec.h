/* Tov: the matcher's state, shared by libregexp.c's interpreter and the matchers `tov build`
 * compiles from a pattern's bytecode (see crates/tov/src/codegen/rejit.rs): the same layout on
 * both sides, so a compiled matcher backtracks through the same stack. */
#ifndef LIBREGEXP_EXEC_H
#define LIBREGEXP_EXEC_H

#include <stdint.h>
#include <stddef.h>

typedef enum {
    RE_EXEC_STATE_SPLIT,
    RE_EXEC_STATE_LOOKAHEAD,
    RE_EXEC_STATE_NEGATIVE_LOOKAHEAD,
    RE_EXEC_STATE_GREEDY, /* Tov: greedy_char's characters to give back (the least run below) */
} REExecStateEnum;

#if INTPTR_MAX >= INT64_MAX
#define BP_TYPE_BITS 3
#else
#define BP_TYPE_BITS 2
#endif

typedef union {
    uint8_t *ptr;
    intptr_t val; /* for bp, the low BP_SHIFT bits store REExecStateEnum */
    struct {
        uintptr_t val : sizeof(uintptr_t) * 8 - BP_TYPE_BITS;
        uintptr_t type : BP_TYPE_BITS;
    } bp;
} StackElem;

typedef struct {
    const uint8_t *cbuf;
    const uint8_t *cbuf_end;
    /* 0 = 8 bit chars, 1 = 16 bit chars, 2 = 16 bit chars, UTF-16 */
    int cbuf_type;
    int capture_count;
    int is_unicode;
    int interrupt_counter;
    void *opaque; /* used for stack overflow check */

    StackElem *stack_buf;
    size_t stack_size;
    StackElem static_stack_buf[512]; /* static stack to avoid allocation in most cases (Tov: 512, not 32: a pattern of a few groups and quantifiers outgrew 32 on every match) */
} REExecContext;

/* A compiled matcher: lre_exec_backtrack's work for one pattern, on 8-bit text. */
typedef intptr_t (*lre_jit_fn)(REExecContext *s, uint8_t **capture, const uint8_t *cptr);

/* Room for n more stack elements (moving the stack): 0, or -1 out of memory. */
int lre_jit_stack_grow(REExecContext *s, size_t n);

/* lre_exec, by a compiled matcher (8-bit text: cbuf_type 0). */
int lre_exec_jit(lre_jit_fn fn, uint8_t **capture, const uint8_t *bc_buf, const uint8_t *cbuf, int cindex, int clen, void *opaque);

#endif
