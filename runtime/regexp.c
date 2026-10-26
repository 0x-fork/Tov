/* Regular expressions: JavaScript's, compiled and run by QuickJS's libregexp (vendor/quickjs-regexp).
 * Linked only into programs that use them. The prelude's RegExp class and string methods are
 * written in Tov over these natives; positions are UTF-16 code units, as in JavaScript (an ASCII
 * string is matched as its bytes, anything else as UTF-16). */
#include "tov.h"
#include "libregexp.h"
#include "libregexp-exec.h"
#include <stdio.h>
#include <stdlib.h>

int lre_check_stack_overflow(void *opaque, size_t alloca_size) {
    (void)opaque;
    (void)alloca_size;
    return 0; /* programs run on a 1 GiB stack */
}

int lre_check_timeout(void *opaque) {
    (void)opaque;
    return 0;
}

void *lre_realloc(void *opaque, void *ptr, size_t size) {
    (void)opaque;
    if (size == 0) {
        free(ptr);
        return NULL;
    }
    return realloc(ptr, size);
}

/* Compiled expressions, kept for the program's life and found again by their source and flags:
 * a literal in a loop compiles once. */
typedef struct tv_re {
    uint8_t *bc;
    tv_str key;      /* flags, '/', source */
    int ncap;
    int nalloc;      /* the capture array's length: 2 * ncap, and the engine's loop counters after */
    int flags;
    char flag_text[12];
    /* One character of a set, or a run of them (`\s+`, `[^a-z]`, `,`): on ASCII text, found by
     * a table instead of the engine. */
    bool simple, plus;
    bool set[128];
    /* On ASCII text, where a match can start: one of the bytes in `first` (`lead`: or with
     * `anchored`, only at the start, or a line's with the m flag), tried there alone by `bc1`,
     * the expression compiled sticky; NULL when it can start with anything. */
    uint8_t *bc1;
    bool anchored;
    bool afirst;         /* anchored, and `first` holds the bytes a match can start with */
    int lead;            /* the one byte `first` holds, or -1 */
    int simple_one;      /* the one byte `set` holds, or -1 */
    tv_arr names;        /* reNames' answer, made once (p NULL until then) */
    bool first[128];
    struct tv_re_lin *lin; /* a sequence matched without the engine, or NULL (see tv_re_lin_parse) */
    /* bc's and bc1's matchers compiled by `tov build` (for ASCII text), or NULL: see tv_rejit_add */
    lre_jit_fn jit, jit1;
    /* the JIT tier (see tv_re_pcre_build): pstate 0 not tried yet, 1 compiled, 2 not for it */
    void *pc, *md;
    int uses;
    uint8_t pstate;
    bool fixup;
} tv_re;

/* The matchers `tov build` compiled from the program's patterns: (key, the bytecode each was
 * compiled from, the matcher), bc's and bc1's. One is used only for the very bytecode it was
 * made from. */
typedef struct {
    const char *key; size_t klen;
    const uint8_t *bc; size_t bclen; lre_jit_fn fn;
    const uint8_t *bc1; size_t bc1len; lre_jit_fn fn1;
} tv_rejit;
static const tv_rejit *tv_rejits;
static size_t tv_nrejits;

void tv_rejit_table(const void *table, size_t n) {
    tv_rejits = table;
    tv_nrejits = n;
}

static size_t tv_bc_size(const uint8_t *bc) {
    return 8 + (size_t)((uint32_t)bc[4] | (uint32_t)bc[5] << 8 | (uint32_t)bc[6] << 16 | (uint32_t)bc[7] << 24);
}

/* A sequence of characters, classes and greedy quantifiers, groups around parts of it (none
 * quantified), `^` first and `$` last at most: matched by taking each item's longest run and,
 * when what follows fails, giving back one character at a time from the nearest item that can
 * spare one, as the engine backtracks. When nothing a quantifier may take could also start what
 * follows it (`det`), the longest runs are the only ones a match can use: no backtracking. */
enum { TV_LIN_ITEMS = 32, TV_LIN_GROUPS = 9 };
typedef struct tv_re_lin {
    int n, ngroups;
    bool start, end; /* `^`, `$` */
    bool any_start;  /* the first item may be empty: a match may start anywhere */
    bool det;        /* every quantifier apart from what may follow it: no backtracking */
    int one;         /* the one character the first item takes, or -1 */
    struct { bool set[128]; int min, max; } item[TV_LIN_ITEMS]; /* max -1: no limit */
    struct { int first, last; } group[TV_LIN_GROUPS];          /* items [first, last) */
} tv_re_lin;

/* The set of one escape (`\s`, `\d`, `\w` and their negations), or false when `c` isn't one. */
static bool tv_re_class_escape(char c, bool set[128]) {
    bool neg = c == 'S' || c == 'D' || c == 'W';
    char k = (char)(neg ? c + 32 : c);
    if (k != 's' && k != 'd' && k != 'w') return false;
    for (int i = 0; i < 128; i++) {
        bool in = k == 's' ? (i == ' ' || (i >= 9 && i <= 13)) : k == 'd' ? (i >= '0' && i <= '9') : (i == '_' || (i >= '0' && i <= '9') || ((i | 32) >= 'a' && (i | 32) <= 'z'));
        if (in != neg) set[i] = true;
    }
    return true;
}

/* A literal character after `\` (`\n`, `\.`), or -1. */
static int tv_re_escaped_char(char c) {
    switch (c) {
    case 'n': return '\n';
    case 't': return '\t';
    case 'r': return '\r';
    case 'f': return '\f';
    case 'v': return '\v';
    }
    if ((unsigned char)c < 128 && !((c >= '0' && c <= '9') || ((c | 32) >= 'a' && (c | 32) <= 'z'))) return (unsigned char)c;
    return -1;
}

/* The class at src[*i] (`[...]`, *i past it after) as its ASCII characters; false when it has
 * anything else (non-ASCII, a `v`-flag set operation) or doesn't parse. */
static bool tv_re_class_at(const tv_re *r, const char *src, size_t n, size_t *ip, bool set[128]) {
    if (r->flags & LRE_FLAG_UNICODE_SETS) return false; /* (`v`: classes have set operations) */
    size_t i = *ip + 1;
    bool neg = i < n && src[i] == '^';
    if (neg) i++;
    bool pos[128] = {0}, any = false;
    while (i < n && src[i] != ']') {
        int lo;
        if ((unsigned char)src[i] >= 128) return false;
        if (src[i] == '\\') {
            if (i + 1 >= n) return false;
            if (tv_re_class_escape(src[i + 1], pos)) { i += 2; any = true; continue; }
            lo = src[i + 1] == 'b' ? '\b' : tv_re_escaped_char(src[i + 1]);
            if (lo < 0) return false;
            i += 2;
        } else {
            lo = (unsigned char)src[i++];
        }
        int hi = lo;
        if (i + 1 < n && src[i] == '-' && src[i + 1] != ']') {
            i++;
            if ((unsigned char)src[i] >= 128) return false;
            if (src[i] == '\\') {
                if (i + 1 >= n) return false;
                hi = tv_re_escaped_char(src[i + 1]);
                if (hi < 0) return false;
                i += 2;
            } else {
                hi = (unsigned char)src[i++];
            }
            if (hi < lo) return false;
        }
        for (int c = lo; c <= hi; c++) pos[c] = true;
        any = true;
    }
    if (i >= n || !any) return false;
    i++; /* ] */
    if (r->flags & LRE_FLAG_IGNORECASE)
        for (int c = 'a'; c <= 'z'; c++) if (pos[c] || pos[c - 32]) pos[c] = pos[c - 32] = true;
    for (int c = 0; c < 128; c++) set[c] = pos[c] != neg;
    *ip = i;
    return true;
}

/* Is the expression one atom (a class, an escape, `.` or a character), maybe with `+`? Then its
 * ASCII characters into r->set. Anything else (groups, anchors, alternatives, other quantifiers,
 * sticky matching, non-ASCII source) is the engine's. */
static bool tv_re_simple_parse(tv_re *r, const char *src, size_t n) {
    if (r->flags & LRE_FLAG_STICKY) return false;
    bool set[128] = {0};
    size_t i = 0;
    if (n == 0) return false;
    for (size_t k = 0; k < n; k++) if ((unsigned char)src[k] >= 128) return false;
    if (src[0] == '\\') {
        if (n < 2) return false;
        if (!tv_re_class_escape(src[1], set)) {
            int c = tv_re_escaped_char(src[1]);
            if (c < 0) return false;
            set[c] = true;
        }
        i = 2;
    } else if (src[0] == '[') {
        i = 0;
        if (!tv_re_class_at(r, src, n, &i, set)) return false;
    } else if (src[0] == '.') {
        for (int c = 0; c < 128; c++) set[c] = (r->flags & LRE_FLAG_DOTALL) || (c != '\n' && c != '\r');
        i = 1;
    } else if (strchr("()|^$*+?{}[]", src[0])) {
        return false;
    } else {
        set[(unsigned char)src[0]] = true;
        i = 1;
    }
    if (r->flags & LRE_FLAG_IGNORECASE)
        for (int c = 'a'; c <= 'z'; c++) if (set[c] || set[c - 32]) set[c] = set[c - 32] = true;
    bool plus = false;
    if (i < n && src[i] == '+') { plus = true; i++; }
    if (i != n) return false;
    memcpy(r->set, set, sizeof set);
    r->plus = plus;
    /* (one character: found with memchr) */
    int count = 0, one = -1;
    for (int c = 0; c < 128; c++) if (set[c]) { count++; one = c; }
    r->simple_one = count == 1 ? one : -1;
    return true;
}

/* src as a tv_re_lin, or false when it isn't one. */
static bool tv_re_lin_parse(const tv_re *r, const char *src, size_t n, tv_re_lin *L) {
    if (r->flags & (LRE_FLAG_STICKY | LRE_FLAG_MULTILINE)) return false;
    memset(L, 0, sizeof *L);
    int open[TV_LIN_GROUPS + 1], depth = 0; /* group index of each open group, -1: (?: ) */
    size_t i = 0;
    if (i < n && src[i] == '^') { L->start = true; i++; }
    while (i < n) {
        char c = src[i];
        if ((unsigned char)c >= 128) return false;
        if (c == '$') {
            if (i + 1 != n || depth) return false;
            L->end = true;
            i++;
            break;
        }
        if (c == '(') {
            if (depth == TV_LIN_GROUPS) return false;
            if (i + 1 < n && src[i + 1] == '?') {
                if (i + 2 >= n || src[i + 2] != ':') return false;
                open[depth++] = -1;
                i += 3;
            } else {
                if (L->ngroups == TV_LIN_GROUPS) return false;
                L->group[L->ngroups].first = L->n;
                open[depth++] = L->ngroups++;
                i++;
            }
            continue;
        }
        if (c == ')') {
            if (!depth) return false;
            int g = open[--depth];
            if (g >= 0) L->group[g].last = L->n;
            i++;
            if (i < n && strchr("*+?{", src[i])) return false; /* (a quantified group) */
            continue;
        }
        if (L->n == TV_LIN_ITEMS) return false;
        bool *set = L->item[L->n].set;
        if (c == '\\') {
            if (i + 1 >= n) return false;
            if (!tv_re_class_escape(src[i + 1], set)) {
                int e = tv_re_escaped_char(src[i + 1]);
                if (e < 0) return false;
                set[e] = true;
            }
            i += 2;
        } else if (c == '[') {
            if (!tv_re_class_at(r, src, n, &i, set)) return false;
        } else if (c == '.') {
            for (int k = 0; k < 128; k++) set[k] = (r->flags & LRE_FLAG_DOTALL) || (k != '\n' && k != '\r');
            i++;
        } else if (strchr("|^*+?{", c)) {
            return false;
        } else {
            set[(unsigned char)c] = true;
            i++;
        }
        if (r->flags & LRE_FLAG_IGNORECASE)
            for (int k = 'a'; k <= 'z'; k++) if (set[k] || set[k - 32]) set[k] = set[k - 32] = true;
        int min = 1, max = 1;
        if (i < n && (src[i] == '*' || src[i] == '+' || src[i] == '?')) {
            min = src[i] == '+' ? 1 : 0;
            max = src[i] == '?' ? 1 : -1;
            i++;
        } else if (i < n && src[i] == '{') {
            size_t j = i + 1;
            int lo = 0, hi;
            if (j >= n || src[j] < '0' || src[j] > '9') return false;
            while (j < n && src[j] >= '0' && src[j] <= '9') { lo = lo * 10 + (src[j++] - '0'); if (lo > 1000) return false; }
            hi = lo;
            if (j < n && src[j] == ',') {
                j++;
                if (j < n && src[j] == '}') hi = -1;
                else {
                    hi = 0;
                    if (j >= n || src[j] < '0' || src[j] > '9') return false;
                    while (j < n && src[j] >= '0' && src[j] <= '9') { hi = hi * 10 + (src[j++] - '0'); if (hi > 1000) return false; }
                    if (hi < lo) return false;
                }
            }
            if (j >= n || src[j] != '}') return false;
            i = j + 1;
            min = lo, max = hi;
        }
        if (i < n && src[i] == '?') return false; /* (lazy) */
        L->item[L->n].min = min;
        L->item[L->n].max = max;
        L->n++;
    }
    if (depth || i != n) return false;
    L->any_start = L->start || L->n == 0 || L->item[0].min == 0;
    L->one = -1;
    if (!L->any_start) {
        int count = 0;
        for (int ch = 0; ch < 128; ch++) if (L->item[0].set[ch]) { L->one = ch; count++; }
        if (count != 1) L->one = -1;
    }
    /* (each quantifier's characters apart from all that may come next: no backtracking) */
    L->det = true;
    for (int k = 0; k < L->n && L->det; k++) {
        if (L->item[k].min == L->item[k].max) continue;
        for (int j = k + 1; j < L->n && L->det; j++) {
            for (int ch = 0; ch < 128; ch++) if (L->item[k].set[ch] && L->item[j].set[ch]) { L->det = false; break; }
            if (L->item[j].min > 0) break;
        }
    }
    return true;
}

/* The first match of L at or after from in buf (ASCII), its groups into cap. */
static bool tv_re_lin_find(const tv_re_lin *L, uint8_t **cap, const uint8_t *buf, tv_int n, tv_int from) {
    const uint8_t *at[TV_LIN_ITEMS + 1];
    /* (where it can start: where its first item can, unless it may be empty; at 0 alone after `^`) */
    if (L->start && from > 0) return false;
    for (tv_int i = from; i <= n; i++) {
        if (L->start && i > 0) return false;
        if (!L->any_start) {
            if (L->one >= 0) {
                const uint8_t *q = i < n ? memchr(buf + i, L->one, (size_t)(n - i)) : NULL;
                if (!q) return false;
                i = (tv_int)(q - buf);
            } else {
                while (i < n && !L->item[0].set[buf[i]]) i++;
                if (i >= n) return false;
            }
        }
        tv_int p = i;
        int k = 0;
        int got[TV_LIN_ITEMS];
        for (;;) {
            /* forward: each item from k on its longest run */
            for (; k < L->n; k++) {
                at[k] = buf + p;
                const bool *set = L->item[k].set;
                int max = L->item[k].max, g = 0;
                while ((max < 0 || g < max) && p < n && set[buf[p]]) p++, g++;
                got[k] = g;
                if (g < L->item[k].min) break;
            }
            if (k == L->n && (!L->end || p == n)) break; /* matched */
            if (L->det) { k = -1; break; }
            /* back: the nearest earlier item that can give a character back (the failing one can't) */
            if (k == L->n) k--;
            else k--;
            while (k >= 0 && got[k] <= L->item[k].min) k--;
            if (k < 0) break;
            got[k]--;
            p = (tv_int)(at[k] - buf) + got[k];
            k++;
        }
        if (k < 0) continue;
        at[L->n] = buf + p;
        cap[0] = (uint8_t *)buf + i;
        cap[1] = (uint8_t *)buf + p;
        for (int g = 0; g < L->ngroups; g++) {
            cap[2 + 2 * g] = (uint8_t *)at[L->group[g].first];
            cap[3 + 2 * g] = (uint8_t *)at[L->group[g].last];
        }
        return true;
    }
    return false;
}

/* Past the rest of a branch: to the `|` or `)` that ends it (not taken), or the end. */
static void tv_re_skip_branch(const char *src, size_t n, size_t *ip) {
    size_t i = *ip;
    int depth = 0;
    while (i < n) {
        char c = src[i];
        if (c == '\\') { i += 2; continue; }
        if (c == '[') {
            /* (`[]` and `[^]` are whole classes in JavaScript) */
            i++;
            if (i < n && src[i] == '^') i++;
            while (i < n && src[i] != ']') i += src[i] == '\\' ? 2 : 1;
            i++;
            continue;
        }
        if (c == '(') depth++;
        else if (c == ')') { if (depth == 0) break; depth--; }
        else if (c == '|' && depth == 0) break;
        i++;
    }
    *ip = i > n ? n : i;
}

/* The bytes the alternatives at src[*i] can start with, into set (*i past them: at the `)` that
 * ends them, or the end); false when one can be empty or start with something not known here. */
/* Past a quantifier at src[*ip] (`?`, `*`, `+`, `{m}`, `{m,}`, `{m,n}`, each lazy too): whether
 * it lets its atom match nothing; no quantifier: false. */
static bool tv_re_skip_quant(const char *src, size_t n, size_t *ip) {
    size_t i = *ip;
    bool nullable = false;
    if (i < n && (src[i] == '?' || src[i] == '*')) {
        nullable = true;
        i++;
    } else if (i < n && src[i] == '+') {
        i++;
    } else if (i < n && src[i] == '{') {
        size_t j = i + 1;
        if (j < n && src[j] >= '0' && src[j] <= '9') {
            bool zero = src[j] == '0';
            while (j < n && src[j] >= '0' && src[j] <= '9') { if (src[j] != '0') zero = false; j++; }
            if (j < n && src[j] == ',') { j++; while (j < n && src[j] >= '0' && src[j] <= '9') j++; }
            if (j < n && src[j] == '}') {
                nullable = zero;
                i = j + 1;
            }
        }
    } else {
        return false;
    }
    if (i < n && src[i] == '?') i++;
    *ip = i;
    return nullable;
}

/* The bytes the alternatives at src[*i] can start with, into set (*i past them: at the `)` that
 * ends them, or the end); false when one can be empty or start with something not known here.
 * Within a branch, an item that may match nothing (`a?`, ` {0,3}`, a lookaround, `\b`) adds its
 * bytes and the next item's. */
static bool tv_re_first_alts(const tv_re *r, const char *src, size_t n, size_t *ip, bool set[128], int depth) {
    if (depth > 8) return false;
    size_t i = *ip;
    for (;;) {
        for (;;) {
            bool atom[128] = {0};
            if (i >= n || src[i] == '|' || src[i] == ')') return false; /* (a branch that can be empty) */
            unsigned char c = (unsigned char)src[i];
            if (c >= 128) return false;
            bool zero_width = false;
            if (c == '(') {
                if (i + 2 < n && src[i + 1] == '?' && (src[i + 2] == '=' || src[i + 2] == '!' || (src[i + 2] == '<' && i + 3 < n && (src[i + 3] == '=' || src[i + 3] == '!')))) {
                    /* (a lookaround: matches no characters; what follows starts the match) */
                    size_t j = i + 1;
                    tv_re_skip_branch(src, n, &j);
                    while (j < n && src[j] == '|') { j++; tv_re_skip_branch(src, n, &j); }
                    if (j >= n || src[j] != ')') return false;
                    i = j + 1;
                    zero_width = true;
                } else {
                    if (i + 2 < n && src[i + 1] == '?') {
                        if (src[i + 2] == ':') i += 3;
                        else if (src[i + 2] == '<') {
                            const char *e = memchr(src + i, '>', n - i);
                            if (!e) return false;
                            i = (size_t)(e - src) + 1;
                        } else {
                            return false;
                        }
                    } else {
                        i++;
                    }
                    if (!tv_re_first_alts(r, src, n, &i, atom, depth + 1)) return false;
                    if (i >= n || src[i] != ')') return false;
                    i++;
                }
            } else if (c == '[') {
                if (!tv_re_class_at(r, src, n, &i, atom)) return false;
            } else if (c == '\\') {
                if (i + 1 >= n) return false;
                if (src[i + 1] == 'b' || src[i + 1] == 'B') {
                    i += 2;
                    zero_width = true;
                } else {
                    if (!tv_re_class_escape(src[i + 1], atom)) {
                        int e = tv_re_escaped_char(src[i + 1]);
                        if (e < 0) return false; /* (back references, \u, \x, \p, ...) */
                        atom[e] = true;
                    }
                    i += 2;
                }
            } else if (strchr(".^$*+?{}|)]", (char)c)) {
                return false;
            } else {
                atom[c] = true;
                i++;
            }
            if (zero_width) continue;
            bool nullable = tv_re_skip_quant(src, n, &i);
            if (r->flags & LRE_FLAG_IGNORECASE)
                for (int k = 'a'; k <= 'z'; k++) if (atom[k] || atom[k - 32]) atom[k] = atom[k - 32] = true;
            for (int k = 0; k < 128; k++) set[k] |= atom[k];
            if (!nullable) break;
        }
        tv_re_skip_branch(src, n, &i);
        if (i < n && src[i] == '|') {
            i++;
            continue;
        }
        *ip = i;
        return true;
    }
}

/* Where a match of the expression can start (see tv_re's bc1), set into r; false when it can
 * start anywhere, as far as this can tell. */
static bool tv_re_first_parse(tv_re *r, const char *src, size_t n) {
    if (r->flags & LRE_FLAG_STICKY) return false;
    for (size_t k = 0; k < n; k++) if ((unsigned char)src[k] >= 128) return false;
    if (n > 0 && src[0] == '^') {
        /* (`^a|^b`: every alternative anchored; `^a|b` isn't) */
        size_t i = 0;
        char *rest = malloc(n + 1);
        size_t rn = 0;
        for (;;) {
            if (i >= n || src[i] != '^') { free(rest); return false; }
            i++;
            size_t b = i;
            tv_re_skip_branch(src, n, &i);
            memcpy(rest + rn, src + b, i - b);
            rn += i - b;
            if (i >= n) break;
            if (src[i] != '|') { free(rest); return false; }
            rest[rn++] = '|';
            i++;
        }
        r->anchored = true;
        /* (and what the alternatives after their `^` can start with: a start that can't, no try) */
        bool set[128] = {0};
        size_t j = 0;
        if (!(r->flags & LRE_FLAG_MULTILINE) && tv_re_first_alts(r, rest, rn, &j, set, 0) && j == rn) {
            memcpy(r->first, set, sizeof set);
            r->afirst = true;
        }
        free(rest);
        return true;
    }
    bool set[128] = {0};
    size_t i = 0;
    if (!tv_re_first_alts(r, src, n, &i, set, 0) || i != n) return false;
    int count = 0, one = -1;
    for (int k = 0; k < 128; k++) if (set[k]) { count++; one = k; }
    if (count == 0 || count > 64) return false;
    memcpy(r->first, set, sizeof set);
    r->lead = count == 1 ? one : -1;
    return true;
}

/* ------------------------------------------------------------------ the JIT tier (PCRE2)
 *
 * An expression used a few times on ASCII text is translated into PCRE2's syntax and compiled
 * by its JIT (vendor/pcre2), when that matches exactly as JavaScript does; else it stays with
 * libregexp. On ASCII text every character an atom may match is one of 128, so each class,
 * escape and literal becomes an explicit set (classes found by asking libregexp itself about
 * each of the 128), and the anchors become what JavaScript means by them: nothing of PCRE2's
 * own reading of a pattern is left. Where the two engines differ is in loops:
 *  - JavaScript rejects an iteration that matches nothing (when no more are required) and goes
 *    on backtracking into it, where PCRE2 leaves the loop: the same once the empty way through
 *    the loop's body is its last one (`empty_last` below), and else the expression stays.
 *  - JavaScript clears a loop body's groups at each iteration, PCRE2 keeps the last values:
 *    the match is the same (unless a backreference reads such a group, then the expression
 *    stays), and the groups are found again by libregexp at the match's start (`fixup`).
 *  - A backreference to a group in a loop must read the group set in the same iteration.
 * Lookbehinds holding groups or backreferences, quantified lookarounds, modifiers, and bounds
 * past PCRE2's stay with libregexp too. */
#define PCRE2_CODE_UNIT_WIDTH 8
#define PCRE2_STATIC 1
#include "pcre2.h"

enum { RN_EMPTY, RN_SET, RN_SEQ, RN_ALT, RN_GROUP, RN_LOOK, RN_QUANT, RN_ASSERT, RN_BACKREF };
typedef struct {
    uint8_t kind;
    bool lazy, neg, behind;
    uint8_t akind;      /* RN_ASSERT: 0 ^, 1 $, 2 \b, 3 \B */
    int cap;            /* RN_GROUP: capture number, 0 when not capturing; RN_BACKREF: the group */
    int min, max;       /* RN_QUANT (max -1: no limit) */
    int child, next, parent;
    int last;           /* the last node of this one's subtree (nodes are numbered in source order) */
    bool nullable, empty_last, consumes;
    int caps;           /* capture groups in the subtree, this node included */
    uint64_t set[2];
} tv_rn;

typedef struct {
    const unsigned char *s;
    size_t n, i;
    bool ic, u, v, m, dotall;
    tv_rn *nodes;
    int nn, capn;
    int ngroups, group_seen;
    const unsigned char **names; /* each group's name (NULL when unnamed), names_len[k] bytes */
    int *names_len;
    bool has_names;
    bool fail;
} tv_rp;

#define TV_RP_FAIL(p) ((p)->fail = true)
static int tv_rp_node(tv_rp *p, int kind) {
    if (p->nn == p->capn) {
        p->capn = p->capn ? p->capn * 2 : 64;
        p->nodes = tv_realloc(p->nodes, (size_t)p->capn * sizeof(tv_rn));
    }
    tv_rn *x = &p->nodes[p->nn];
    memset(x, 0, sizeof *x);
    x->kind = (uint8_t)kind;
    x->child = x->next = x->parent = -1;
    return p->nn++;
}

static void tv_rp_add(tv_rp *p, int parent, int *tail, int child) {
    p->nodes[child].parent = parent;
    if (*tail < 0) p->nodes[parent].child = child;
    else p->nodes[*tail].next = child;
    *tail = child;
}

static void tv_set_add(uint64_t set[2], int c) {
    if (c >= 0 && c < 128) set[c >> 6] |= 1ull << (c & 63);
}

static bool tv_set_has(const uint64_t set[2], int c) {
    return (set[c >> 6] >> (c & 63)) & 1;
}

/* A literal character's set on ASCII text (`cp` a code point, or a UTF-16 unit without u). */
static void tv_rp_char_set(const tv_rp *p, uint32_t cp, uint64_t set[2]) {
    set[0] = set[1] = 0;
    if (cp < 128) {
        tv_set_add(set, (int)cp);
        if (p->ic && ((cp | 32) >= 'a' && (cp | 32) <= 'z')) tv_set_add(set, (int)(cp ^ 32));
    } else if (p->ic && (p->u || p->v)) {
        /* (with u, case folding: the long s and the Kelvin sign fold to ASCII letters) */
        if (cp == 0x17F) tv_set_add(set, 's'), tv_set_add(set, 'S');
        if (cp == 0x212A) tv_set_add(set, 'k'), tv_set_add(set, 'K');
    }
}

/* What libregexp itself matches of the 128 ASCII characters with the atom text[0, len) (a class
 * or an escape), with the expression's flags: false when it can't say. Answers are kept. */
typedef struct { char *text; size_t len; int flags; uint64_t set[2]; } tv_probe_entry;
static tv_probe_entry *tv_probes;
static size_t tv_nprobes, tv_cap_probes;

static bool tv_re_probe(const char *text, size_t len, int flags, uint64_t set[2]) {
    flags &= LRE_FLAG_IGNORECASE | LRE_FLAG_UNICODE | LRE_FLAG_UNICODE_SETS;
    for (size_t k = 0; k < tv_nprobes; k++) {
        tv_probe_entry *e = &tv_probes[k];
        if (e->len == len && e->flags == flags && !memcmp(e->text, text, len)) {
            set[0] = e->set[0], set[1] = e->set[1];
            return true;
        }
    }
    char err[64];
    int blen;
    char *z = tv_alloc(len + 1); /* (libregexp reads up to a NUL) */
    memcpy(z, text, len);
    z[len] = 0;
    uint8_t *bc = lre_compile(&blen, err, sizeof err, z, len, flags | LRE_FLAG_STICKY, NULL);
    if (!bc || lre_get_capture_count(bc) != 1) {
        free(bc);
        tv_free(z);
        return false;
    }
    static uint8_t all[128];
    for (int c = 0; c < 128; c++) all[c] = (uint8_t)c;
    set[0] = set[1] = 0;
    bool ok = true;
    uint8_t **cap = tv_alloc((size_t)lre_get_alloc_count(bc) * sizeof(uint8_t *) + sizeof(uint8_t *));
    for (int c = 0; c < 128 && ok; c++) {
        int ret = lre_exec(cap, bc, all, c, 128, 0, NULL);
        if (ret == 1) {
            if (cap[1] - cap[0] != 1) ok = false; /* (a class of strings: `\q{ab}`) */
            else tv_set_add(set, c);
        } else if (ret < 0) {
            ok = false;
        }
    }
    free(bc);
    tv_free(cap);
    if (!ok) {
        tv_free(z);
        return false;
    }
    if (tv_nprobes == tv_cap_probes) {
        tv_cap_probes = tv_cap_probes ? tv_cap_probes * 2 : 32;
        tv_probes = tv_realloc(tv_probes, tv_cap_probes * sizeof *tv_probes);
    }
    tv_probe_entry *e = &tv_probes[tv_nprobes++];
    e->text = z;
    e->len = len;
    e->flags = flags;
    e->set[0] = set[0], e->set[1] = set[1];
    return true;
}

static int tv_rp_hex(int c) {
    if (c >= '0' && c <= '9') return c - '0';
    if ((c | 32) >= 'a' && (c | 32) <= 'f') return (c | 32) - 'a' + 10;
    return -1;
}

/* n hex digits at i, or -1 */
static long tv_rp_hexn(const tv_rp *p, size_t i, int n) {
    long v = 0;
    for (int k = 0; k < n; k++) {
        if (i + (size_t)k >= p->n) return -1;
        int d = tv_rp_hex(p->s[i + (size_t)k]);
        if (d < 0) return -1;
        v = v * 16 + d;
    }
    return v;
}

/* A UTF-8 sequence at i: its code point, *len its bytes */
static uint32_t tv_rp_utf8(const tv_rp *p, size_t i, size_t *len) {
    const unsigned char *d = p->s + i;
    uint32_t c = d[0];
    size_t l = 1;
    if (c >= 0xF0 && i + 3 < p->n) c = ((c & 7) << 18) | ((d[1] & 63u) << 12) | ((d[2] & 63u) << 6) | (d[3] & 63u), l = 4;
    else if (c >= 0xE0 && i + 2 < p->n) c = ((c & 15) << 12) | ((d[1] & 63u) << 6) | (d[2] & 63u), l = 3;
    else if (c >= 0xC0 && i + 1 < p->n) c = ((c & 31) << 6) | (d[1] & 63u), l = 2;
    *len = l;
    return c;
}

static int tv_rp_set_node(tv_rp *p, const uint64_t set[2]) {
    int x = tv_rp_node(p, RN_SET);
    p->nodes[x].set[0] = set[0];
    p->nodes[x].set[1] = set[1];
    return x;
}

/* The group a name stands for (1-based), or -1 (also when two groups share it) */
static int tv_rp_named(const tv_rp *p, const unsigned char *name, size_t len) {
    int found = -1;
    for (int k = 1; k <= p->ngroups; k++) {
        if (p->names[k] && (size_t)p->names_len[k] == len && !memcmp(p->names[k], name, len)) {
            if (found >= 0) return -1;
            found = k;
        }
    }
    return found;
}

/* The groups and their names, before parsing (a backreference may come before its group). */
static void tv_rp_scan_groups(tv_rp *p) {
    int count = 0;
    for (size_t pass = 0; pass < 2; pass++) {
        int k = 0;
        int depth = 0; /* inside a class (nested with v) */
        for (size_t i = 0; i < p->n; i++) {
            unsigned char c = p->s[i];
            if (c == '\\') { i++; continue; }
            if (depth) {
                if (c == ']') depth--;
                else if (c == '[' && p->v) depth++;
                continue;
            }
            if (c == '[') { depth = 1; continue; }
            if (c != '(') continue;
            if (i + 1 < p->n && p->s[i + 1] == '?') {
                if (i + 2 < p->n && p->s[i + 2] == '<' && i + 3 < p->n && p->s[i + 3] != '=' && p->s[i + 3] != '!') {
                    k++;
                    if (pass) {
                        size_t a = i + 3, b = a;
                        while (b < p->n && p->s[b] != '>') {
                            if (p->s[b] == '\\') TV_RP_FAIL(p); /* (escapes in a group's name) */
                            b++;
                        }
                        p->names[k] = p->s + a;
                        p->names_len[k] = (int)(b - a);
                        p->has_names = true;
                    }
                }
                continue;
            }
            k++;
            if (pass) p->names[k] = NULL;
        }
        if (!pass) {
            count = k;
            p->ngroups = count;
            p->names = tv_alloc((size_t)(count + 1) * sizeof *p->names);
            p->names_len = tv_alloc((size_t)(count + 1) * sizeof *p->names_len);
            memset(p->names, 0, (size_t)(count + 1) * sizeof *p->names);
        }
    }
}

static int tv_rp_alt(tv_rp *p);

/* A quantifier at i: true with its bounds, i past it (not past a lazy `?`) */
static bool tv_rp_quant(tv_rp *p, int *min, int *max) {
    if (p->i >= p->n) return false;
    unsigned char c = p->s[p->i];
    if (c == '*') { *min = 0, *max = -1, p->i++; return true; }
    if (c == '+') { *min = 1, *max = -1, p->i++; return true; }
    if (c == '?') { *min = 0, *max = 1, p->i++; return true; }
    if (c != '{') return false;
    size_t j = p->i + 1;
    long a = 0, b;
    size_t d0 = j;
    while (j < p->n && p->s[j] >= '0' && p->s[j] <= '9') { a = a * 10 + (p->s[j] - '0'); if (a > 1000000) a = 1000000; j++; }
    if (j == d0) return false;
    if (j < p->n && p->s[j] == '}') {
        b = a;
    } else if (j < p->n && p->s[j] == ',') {
        j++;
        size_t d1 = j;
        b = 0;
        while (j < p->n && p->s[j] >= '0' && p->s[j] <= '9') { b = b * 10 + (p->s[j] - '0'); if (b > 1000000) b = 1000000; j++; }
        if (j == d1) b = -1;
        if (j >= p->n || p->s[j] != '}') return false;
    } else {
        return false;
    }
    p->i = j + 1;
    if (a > 65535 || b > 65535) TV_RP_FAIL(p);
    *min = (int)a, *max = (int)b;
    return true;
}

/* An escape after the backslash at p->i - 1: an atom or assertion node, or -1 */
static int tv_rp_escape(tv_rp *p) {
    if (p->i >= p->n) { TV_RP_FAIL(p); return -1; }
    unsigned char c = p->s[p->i];
    uint64_t set[2] = {0, 0};
    size_t at = p->i - 1;
    switch (c) {
    case 'b': case 'B': {
        p->i++;
        int x = tv_rp_node(p, RN_ASSERT);
        p->nodes[x].akind = c == 'b' ? 2 : 3;
        return x;
    }
    case 'd': case 'D': case 'w': case 'W': case 's': case 'S':
        p->i++;
        if (p->ic) {
            if (!tv_re_probe((const char *)p->s + at, 2, (p->ic ? LRE_FLAG_IGNORECASE : 0) | (p->u ? LRE_FLAG_UNICODE : 0) | (p->v ? LRE_FLAG_UNICODE_SETS : 0), set)) { TV_RP_FAIL(p); return -1; }
            return tv_rp_set_node(p, set);
        }
        for (int k = 0; k < 128; k++) {
            bool in = (c | 32) == 'd' ? (k >= '0' && k <= '9')
                    : (c | 32) == 'w' ? ((k | 32) >= 'a' && (k | 32) <= 'z') || (k >= '0' && k <= '9') || k == '_'
                    : (k >= 9 && k <= 13) || k == ' ';
            if (in != (c < 'a')) tv_set_add(set, k);
        }
        return tv_rp_set_node(p, set);
    case 'p': case 'P':
        if (p->u || p->v) {
            size_t j = p->i + 1;
            if (j >= p->n || p->s[j] != '{') { TV_RP_FAIL(p); return -1; }
            while (j < p->n && p->s[j] != '}') j++;
            if (j >= p->n) { TV_RP_FAIL(p); return -1; }
            p->i = j + 1;
            if (!tv_re_probe((const char *)p->s + at, p->i - at, (p->ic ? LRE_FLAG_IGNORECASE : 0) | (p->u ? LRE_FLAG_UNICODE : 0) | (p->v ? LRE_FLAG_UNICODE_SETS : 0), set)) { TV_RP_FAIL(p); return -1; }
            return tv_rp_set_node(p, set);
        }
        break;
    case 'k':
        if (p->u || p->v || p->has_names) {
            if (p->i + 1 >= p->n || p->s[p->i + 1] != '<') { TV_RP_FAIL(p); return -1; }
            size_t a = p->i + 2, b = a;
            while (b < p->n && p->s[b] != '>') b++;
            if (b >= p->n) { TV_RP_FAIL(p); return -1; }
            int g = tv_rp_named(p, p->s + a, b - a);
            if (g < 0) { TV_RP_FAIL(p); return -1; }
            p->i = b + 1;
            int x = tv_rp_node(p, RN_BACKREF);
            p->nodes[x].cap = g;
            return x;
        }
        break;
    case '0':
        if (p->i + 1 < p->n && p->s[p->i + 1] >= '0' && p->s[p->i + 1] <= '9') { TV_RP_FAIL(p); return -1; }
        p->i++;
        tv_rp_char_set(p, 0, set);
        return tv_rp_set_node(p, set);
    case 'f': p->i++; tv_rp_char_set(p, '\f', set); return tv_rp_set_node(p, set);
    case 'n': p->i++; tv_rp_char_set(p, '\n', set); return tv_rp_set_node(p, set);
    case 'r': p->i++; tv_rp_char_set(p, '\r', set); return tv_rp_set_node(p, set);
    case 't': p->i++; tv_rp_char_set(p, '\t', set); return tv_rp_set_node(p, set);
    case 'v': p->i++; tv_rp_char_set(p, '\v', set); return tv_rp_set_node(p, set);
    case 'c':
        if (p->i + 1 < p->n && ((p->s[p->i + 1] | 32) >= 'a' && (p->s[p->i + 1] | 32) <= 'z')) {
            tv_rp_char_set(p, p->s[p->i + 1] & 31, set);
            p->i += 2;
            return tv_rp_set_node(p, set);
        }
        if (p->u || p->v) { TV_RP_FAIL(p); return -1; }
        /* (Annex B: the backslash is itself, and the `c` comes next) */
        tv_rp_char_set(p, '\\', set);
        return tv_rp_set_node(p, set);
    case 'x': {
        long h = tv_rp_hexn(p, p->i + 1, 2);
        if (h >= 0) {
            p->i += 3;
            tv_rp_char_set(p, (uint32_t)h, set);
            return tv_rp_set_node(p, set);
        }
        if (p->u || p->v) { TV_RP_FAIL(p); return -1; }
        break;
    }
    case 'u': {
        if ((p->u || p->v) && p->i + 1 < p->n && p->s[p->i + 1] == '{') {
            size_t j = p->i + 2;
            long cp = 0;
            while (j < p->n && tv_rp_hex(p->s[j]) >= 0) { cp = cp * 16 + tv_rp_hex(p->s[j]); if (cp > 0x10FFFF) cp = 0x110000; j++; }
            if (j >= p->n || p->s[j] != '}' || cp > 0x10FFFF) { TV_RP_FAIL(p); return -1; }
            p->i = j + 1;
            tv_rp_char_set(p, (uint32_t)cp, set);
            return tv_rp_set_node(p, set);
        }
        long h = tv_rp_hexn(p, p->i + 1, 4);
        if (h >= 0) {
            p->i += 5;
            /* (with u, a surrogate pair written as two escapes is one character) */
            if ((p->u || p->v) && h >= 0xD800 && h <= 0xDBFF && p->i + 1 < p->n && p->s[p->i] == '\\' && p->s[p->i + 1] == 'u') {
                long lo = tv_rp_hexn(p, p->i + 2, 4);
                if (lo >= 0xDC00 && lo <= 0xDFFF) {
                    p->i += 6;
                    h = 0x10000 + ((h - 0xD800) << 10) + (lo - 0xDC00);
                }
            }
            tv_rp_char_set(p, (uint32_t)h, set);
            return tv_rp_set_node(p, set);
        }
        if (p->u || p->v) { TV_RP_FAIL(p); return -1; }
        break;
    }
    default:
        if (c >= '1' && c <= '9') {
            size_t j = p->i;
            long g = 0;
            while (j < p->n && p->s[j] >= '0' && p->s[j] <= '9') { g = g * 10 + (p->s[j] - '0'); if (g > 100000) g = 100000; j++; }
            if (g > p->ngroups) { TV_RP_FAIL(p); return -1; } /* (Annex B octal and identity escapes) */
            p->i = j;
            int x = tv_rp_node(p, RN_BACKREF);
            p->nodes[x].cap = (int)g;
            return x;
        }
        break;
    }
    /* the character itself */
    size_t len;
    uint32_t cp = tv_rp_utf8(p, p->i, &len);
    p->i += len;
    if (!(p->u || p->v) && cp >= 0x10000) {
        /* (two UTF-16 units: the first an atom of its own) */
        tv_rp_char_set(p, 0xD800, set);
        int x = tv_rp_set_node(p, set);
        TV_RP_FAIL(p); /* (rare: not worth splitting into two atoms here) */
        return x;
    }
    tv_rp_char_set(p, cp, set);
    return tv_rp_set_node(p, set);
}

/* A term: an assertion, or an atom and its quantifier. -1 at the end of an alternative. */
static int tv_rp_term(tv_rp *p) {
    if (p->i >= p->n || p->fail) return -1;
    unsigned char c = p->s[p->i];
    if (c == '|' || c == ')') return -1;
    int atom = -1;
    bool look = false;
    uint64_t set[2] = {0, 0};
    if (c == '^' || c == '$') {
        p->i++;
        int x = tv_rp_node(p, RN_ASSERT);
        p->nodes[x].akind = c == '^' ? 0 : 1;
        return x;
    } else if (c == '(') {
        p->i++;
        int x;
        if (p->i < p->n && p->s[p->i] == '?') {
            unsigned char d = p->i + 1 < p->n ? p->s[p->i + 1] : 0;
            if (d == ':') {
                x = tv_rp_node(p, RN_GROUP);
                p->i += 2;
            } else if (d == '=' || d == '!') {
                x = tv_rp_node(p, RN_LOOK);
                p->nodes[x].neg = d == '!';
                p->i += 2;
                look = true;
            } else if (d == '<' && p->i + 2 < p->n && (p->s[p->i + 2] == '=' || p->s[p->i + 2] == '!')) {
                x = tv_rp_node(p, RN_LOOK);
                p->nodes[x].neg = p->s[p->i + 2] == '!';
                p->nodes[x].behind = true;
                p->i += 3;
                look = true;
            } else if (d == '<') {
                x = tv_rp_node(p, RN_GROUP);
                p->nodes[x].cap = ++p->group_seen;
                while (p->i < p->n && p->s[p->i] != '>') p->i++;
                p->i++;
            } else {
                TV_RP_FAIL(p); /* (modifiers) */
                return -1;
            }
        } else {
            x = tv_rp_node(p, RN_GROUP);
            p->nodes[x].cap = ++p->group_seen;
        }
        int inner = tv_rp_alt(p);
        if (p->fail || p->i >= p->n || p->s[p->i] != ')') { TV_RP_FAIL(p); return -1; }
        p->i++;
        int tail = -1;
        tv_rp_add(p, x, &tail, inner);
        atom = x;
    } else if (c == '.') {
        p->i++;
        for (int k = 0; k < 128; k++)
            if (p->dotall || (k != '\n' && k != '\r')) tv_set_add(set, k);
        atom = tv_rp_set_node(p, set);
    } else if (c == '[') {
        size_t j = p->i + 1;
        int depth = 1;
        if (j < p->n && p->s[j] == '^') j++;
        while (j < p->n && depth) {
            unsigned char d = p->s[j];
            if (d == '\\') { j += 2; continue; }
            if (d >= 0xF0 && !(p->u || p->v)) { TV_RP_FAIL(p); return -1; } /* (UTF-16 halves) */
            if (d == ']') depth--;
            else if (d == '[' && p->v) depth++;
            j++;
        }
        if (depth) { TV_RP_FAIL(p); return -1; }
        if (!tv_re_probe((const char *)p->s + p->i, j - p->i, (p->ic ? LRE_FLAG_IGNORECASE : 0) | (p->u ? LRE_FLAG_UNICODE : 0) | (p->v ? LRE_FLAG_UNICODE_SETS : 0), set)) { TV_RP_FAIL(p); return -1; }
        p->i = j;
        atom = tv_rp_set_node(p, set);
    } else if (c == '\\') {
        p->i++;
        atom = tv_rp_escape(p);
        if (atom < 0) return -1;
        if (p->nodes[atom].kind == RN_ASSERT) return atom;
    } else if (c == '*' || c == '+' || c == '?') {
        TV_RP_FAIL(p);
        return -1;
    } else {
        size_t len;
        uint32_t cp = tv_rp_utf8(p, p->i, &len);
        if (!(p->u || p->v) && cp >= 0x10000) { TV_RP_FAIL(p); return -1; }
        p->i += len;
        tv_rp_char_set(p, cp, set);
        atom = tv_rp_set_node(p, set);
    }
    int min, max;
    size_t before = p->i;
    if (tv_rp_quant(p, &min, &max)) {
        if (look) { TV_RP_FAIL(p); return -1; } /* (Annex B's quantified lookaheads) */
        int q = tv_rp_node(p, RN_QUANT);
        p->nodes[q].min = min;
        p->nodes[q].max = max;
        if (p->i < p->n && p->s[p->i] == '?') p->nodes[q].lazy = true, p->i++;
        int tail = -1;
        tv_rp_add(p, q, &tail, atom);
        return q;
    }
    p->i = before;
    return atom;
}

static int tv_rp_seq(tv_rp *p) {
    int seq = tv_rp_node(p, RN_SEQ), tail = -1;
    for (;;) {
        int t = tv_rp_term(p);
        if (t < 0) break;
        tv_rp_add(p, seq, &tail, t);
    }
    return seq;
}

static int tv_rp_alt(tv_rp *p) {
    int first = tv_rp_seq(p);
    if (p->fail || p->i >= p->n || p->s[p->i] != '|') return first;
    int alt = tv_rp_node(p, RN_ALT), tail = -1;
    tv_rp_add(p, alt, &tail, first);
    while (!p->fail && p->i < p->n && p->s[p->i] == '|') {
        p->i++;
        tv_rp_add(p, alt, &tail, tv_rp_seq(p));
    }
    return alt;
}

/* nullable, empty_last, consumes, caps and last, bottom-up */
static void tv_rp_analyze(tv_rp *p, int x) {
    tv_rn *n = &p->nodes[x];
    n->last = x;
    n->caps = n->kind == RN_GROUP && n->cap ? 1 : 0;
    for (int c = n->child; c >= 0; c = p->nodes[c].next) {
        tv_rp_analyze(p, c);
        n = &p->nodes[x];
        if (p->nodes[c].last > n->last) n->last = p->nodes[c].last;
        n->caps += p->nodes[c].caps;
    }
    n = &p->nodes[x];
    const tv_rn *k = n->child >= 0 ? &p->nodes[n->child] : NULL;
    switch (n->kind) {
    case RN_EMPTY: case RN_ASSERT: case RN_LOOK: case RN_BACKREF:
        n->nullable = n->empty_last = true;
        n->consumes = n->kind == RN_BACKREF;
        /* (a negative lookaround's groups are never set once it holds, in either engine) */
        if (n->kind == RN_LOOK && n->neg) n->caps = 0;
        break;
    case RN_SET:
        n->nullable = n->empty_last = false;
        n->consumes = true;
        break;
    case RN_GROUP:
        n->nullable = k->nullable, n->empty_last = k->empty_last, n->consumes = k->consumes;
        break;
    case RN_SEQ:
        n->nullable = n->empty_last = true;
        n->consumes = false;
        for (int c = n->child; c >= 0; c = p->nodes[c].next) {
            const tv_rn *e = &p->nodes[c];
            n->nullable &= e->nullable;
            n->empty_last &= e->empty_last;
            n->consumes |= e->consumes;
        }
        break;
    case RN_ALT:
        n->nullable = false;
        n->empty_last = true;
        n->consumes = false;
        for (int c = n->child; c >= 0; c = p->nodes[c].next) {
            const tv_rn *e = &p->nodes[c];
            n->consumes |= e->consumes;
            if (e->nullable) {
                n->nullable = true;
                /* (an alternative that may be empty, with others after it) */
                if (e->next >= 0 || !e->empty_last) n->empty_last = false;
            }
        }
        break;
    case RN_QUANT:
        n->consumes = k->consumes && n->max != 0;
        n->nullable = n->min == 0 || k->nullable;
        if (n->min == 0) n->empty_last = !n->lazy || !n->consumes;
        else n->empty_last = k->empty_last;
        if (k->nullable && (!k->empty_last || k->caps)) TV_RP_FAIL(p);
        break;
    }
}

/* Whether node x lies inside node a's subtree */
static bool tv_rp_within(const tv_rp *p, int x, int a) {
    for (; x >= 0; x = p->nodes[x].parent)
        if (x == a) return true;
    return false;
}

/* The checks over the whole tree: loops' groups (fixup), backreferences, lookbehinds. */
static void tv_rp_check(tv_rp *p, bool *fixup) {
    int *group_node = tv_alloc((size_t)(p->ngroups + 1) * sizeof(int));
    for (int g = 0; g <= p->ngroups; g++) group_node[g] = -1;
    for (int x = 0; x < p->nn; x++)
        if (p->nodes[x].kind == RN_GROUP && p->nodes[x].cap && p->nodes[x].cap <= p->ngroups) group_node[p->nodes[x].cap] = x;
    for (int x = 0; x < p->nn && !p->fail; x++) {
        const tv_rn *n = &p->nodes[x];
        if (n->kind == RN_QUANT && (n->max > 1 || n->max < 0)) {
            const tv_rn *k = &p->nodes[n->child];
            if (k->caps - (k->kind == RN_GROUP && k->cap ? 1 : 0) > 0) *fixup = true;
        }
        if (n->kind == RN_LOOK && n->behind) {
            for (int y = x + 1; y <= n->last; y++)
                if ((p->nodes[y].kind == RN_GROUP && p->nodes[y].cap) || p->nodes[y].kind == RN_BACKREF) TV_RP_FAIL(p);
        }
        if (n->kind != RN_BACKREF) continue;
        int g = group_node[n->cap];
        if (g < 0) { TV_RP_FAIL(p); break; }
        /* the loops around the group */
        bool looped = false;
        for (int a = p->nodes[g].parent; a >= 0; a = p->nodes[a].parent) {
            const tv_rn *q = &p->nodes[a];
            if (q->kind == RN_QUANT && (q->max > 1 || q->max < 0)) {
                looped = true;
                if (!tv_rp_within(p, x, a)) TV_RP_FAIL(p); /* (read after the loop) */
            }
        }
        if (!looped || p->fail) continue;
        /* in the same iteration: after the group, which nothing between their common ancestor
         * and it makes optional */
        if (x <= p->nodes[g].last) { TV_RP_FAIL(p); continue; }
        int lca = p->nodes[g].parent;
        while (lca >= 0 && !tv_rp_within(p, x, lca)) lca = p->nodes[lca].parent;
        for (int a = p->nodes[g].parent; a >= 0 && a != lca; a = p->nodes[a].parent) {
            const tv_rn *q = &p->nodes[a];
            if (q->kind == RN_ALT || (q->kind == RN_QUANT && q->min == 0) || q->kind == RN_LOOK) TV_RP_FAIL(p);
        }
    }
    tv_free(group_node);
}

static void tv_rp_emit_set(tv_sb *sb, const uint64_t set[2]) {
    int count = 0, one = -1;
    for (int c = 0; c < 128; c++)
        if (tv_set_has(set, c)) count++, one = c;
    char tmp[16];
    if (count == 0) {
        tv_sb_push(sb, "[^\\x00-\\xff]", 12);
        return;
    }
    if (count == 1) {
        snprintf(tmp, sizeof tmp, "\\x%02x", one);
        tv_sb_push(sb, tmp, 4);
        return;
    }
    if (count == 128) {
        tv_sb_push(sb, "[\\x00-\\xff]", 11);
        return;
    }
    tv_sb_push_char(sb, '[');
    for (int c = 0; c < 128;) {
        if (!tv_set_has(set, c)) { c++; continue; }
        int d = c;
        while (d + 1 < 128 && tv_set_has(set, d + 1)) d++;
        snprintf(tmp, sizeof tmp, "\\x%02x", c);
        tv_sb_push(sb, tmp, 4);
        if (d > c) {
            snprintf(tmp, sizeof tmp, "-\\x%02x", d);
            tv_sb_push(sb, tmp, 5);
        }
        c = d + 1;
    }
    tv_sb_push_char(sb, ']');
}

static void tv_rp_emit(const tv_rp *p, int x, tv_sb *sb) {
    const tv_rn *n = &p->nodes[x];
    char tmp[48];
    switch (n->kind) {
    case RN_EMPTY: break;
    case RN_SET: tv_rp_emit_set(sb, n->set); break;
    case RN_SEQ:
        for (int c = n->child; c >= 0; c = p->nodes[c].next) tv_rp_emit(p, c, sb);
        break;
    case RN_ALT:
        for (int c = n->child; c >= 0; c = p->nodes[c].next) {
            if (c != n->child) tv_sb_push_char(sb, '|');
            tv_rp_emit(p, c, sb);
        }
        break;
    case RN_GROUP:
        tv_sb_push(sb, n->cap ? "(" : "(?:", n->cap ? 1 : 3);
        tv_rp_emit(p, n->child, sb);
        tv_sb_push_char(sb, ')');
        break;
    case RN_LOOK:
        if (n->behind) tv_sb_push(sb, n->neg ? "(?<!" : "(?<=", 4);
        else tv_sb_push(sb, n->neg ? "(?!" : "(?=", 3);
        tv_rp_emit(p, n->child, sb);
        tv_sb_push_char(sb, ')');
        break;
    case RN_ASSERT:
        switch (n->akind) {
        case 0: if (p->m) tv_sb_push(sb, "(?<![^\\n\\r])", 12); else tv_sb_push(sb, "\\A", 2); break;
        case 1: if (p->m) tv_sb_push(sb, "(?![^\\n\\r])", 11); else tv_sb_push(sb, "\\z", 2); break;
        case 2: tv_sb_push(sb, "\\b", 2); break;
        default: tv_sb_push(sb, "\\B", 2); break;
        }
        break;
    case RN_BACKREF:
        snprintf(tmp, sizeof tmp, p->ic ? "(?i:\\g{%d})" : "(?:\\g{%d})", n->cap);
        tv_sb_push(sb, tmp, strlen(tmp));
        break;
    case RN_QUANT: {
        const tv_rn *k = &p->nodes[n->child];
        bool wrap = !(k->kind == RN_SET || k->kind == RN_GROUP || k->kind == RN_BACKREF);
        if (wrap) tv_sb_push(sb, "(?:", 3);
        tv_rp_emit(p, n->child, sb);
        if (wrap) tv_sb_push_char(sb, ')');
        if (n->max < 0) snprintf(tmp, sizeof tmp, "{%d,}", n->min);
        else snprintf(tmp, sizeof tmp, "{%d,%d}", n->min, n->max);
        tv_sb_push(sb, tmp, strlen(tmp));
        if (n->lazy) tv_sb_push_char(sb, '?');
        break;
    }
    }
}

/* PCRE2's state for the program: one JIT stack for every expression (single-threaded). */
static pcre2_match_context *tv_pcre_mctx;
static pcre2_jit_stack *tv_pcre_stack;
static int tv_pcre_enabled = -1;

/* Compiles r for the JIT tier (r->pstate 1), or gives up on it (2). */
static void tv_re_pcre_build(tv_re *r) {
    r->pstate = 2;
    if (tv_pcre_enabled < 0) {
        const char *e = getenv("TOV_REGEXP_ENGINE");
        tv_pcre_enabled = !(e && !strcmp(e, "interpreter"));
    }
    if (!tv_pcre_enabled) return;
    const char *key = r->key.p->data;
    size_t klen = (size_t)tv_str_byte_len(r->key);
    const char *slash = memchr(key, '/', klen);
    if (!slash) return;
    tv_rp p = {0};
    p.s = (const unsigned char *)slash + 1;
    p.n = klen - (size_t)(slash + 1 - key);
    p.ic = r->flags & LRE_FLAG_IGNORECASE;
    p.u = r->flags & LRE_FLAG_UNICODE;
    p.v = r->flags & LRE_FLAG_UNICODE_SETS;
    p.m = r->flags & LRE_FLAG_MULTILINE;
    p.dotall = r->flags & LRE_FLAG_DOTALL;
    tv_rp_scan_groups(&p);
    int root = p.fail ? -1 : tv_rp_alt(&p);
    if (!p.fail && p.i != p.n) TV_RP_FAIL(&p);
    if (!p.fail && p.ngroups + 1 != r->ncap) TV_RP_FAIL(&p);
    if (!p.fail && p.group_seen != p.ngroups) TV_RP_FAIL(&p);
    bool fixup = false;
    if (!p.fail) tv_rp_analyze(&p, root);
    if (!p.fail) tv_rp_check(&p, &fixup);
    tv_sb sb = {0};
    if (!p.fail) tv_rp_emit(&p, root, &sb);
    tv_free(p.nodes);
    tv_free(p.names);
    tv_free(p.names_len);
    if (p.fail) {
        tv_sb_free(&sb);
        return;
    }
    int err;
    PCRE2_SIZE erroff;
    uint32_t opts = PCRE2_MATCH_UNSET_BACKREF | PCRE2_NEVER_UTF | PCRE2_NEVER_UCP | PCRE2_NEVER_BACKSLASH_C | PCRE2_NO_UTF_CHECK;
    if (r->flags & LRE_FLAG_STICKY) opts |= PCRE2_ANCHORED;
    pcre2_code *code = pcre2_compile((PCRE2_SPTR)(sb.data ? sb.data : ""), sb.len, opts, &err, &erroff, NULL);
    tv_sb_free(&sb);
    if (!code) return;
    uint32_t ncap = 0;
    pcre2_pattern_info(code, PCRE2_INFO_CAPTURECOUNT, &ncap);
    if ((int)ncap + 1 != r->ncap || pcre2_jit_compile(code, PCRE2_JIT_COMPLETE) != 0) {
        pcre2_code_free(code);
        return;
    }
    if (!tv_pcre_mctx) {
        tv_pcre_mctx = pcre2_match_context_create(NULL);
        tv_pcre_stack = pcre2_jit_stack_create(64 * 1024, 64 * 1024 * 1024, NULL);
        if (!tv_pcre_mctx || !tv_pcre_stack) {
            pcre2_code_free(code);
            return;
        }
        pcre2_jit_stack_assign(tv_pcre_mctx, NULL, tv_pcre_stack);
    }
    r->pc = code;
    r->md = pcre2_match_data_create_from_pattern(code, NULL);
    r->fixup = fixup;
    r->pstate = 1;
}

/* After this many matches an expression is compiled for the JIT tier. */
enum { TV_PCRE_AFTER = 3 };

/* The match at or after `from` by the JIT tier: 1 found, 0 none, -1 when it can't say (a limit
 * of PCRE2's reached: libregexp, which has none, answers instead). */
static int tv_re_pcre_find(const tv_re *r, uint8_t **cap, const uint8_t *buf, tv_int n, tv_int from, bool want_groups) {
    pcre2_match_data *md = r->md;
    int rc = pcre2_jit_match(r->pc, buf, (PCRE2_SIZE)n, (PCRE2_SIZE)from, 0, md, tv_pcre_mctx);
    if (rc == PCRE2_ERROR_NOMATCH) return 0;
    if (rc <= 0) return -1;
    PCRE2_SIZE *ov = pcre2_get_ovector_pointer(md);
    if (r->fixup && want_groups && r->ncap > 1) {
        /* (the groups as JavaScript leaves them: found again from where the match starts) */
        int ret = lre_exec(cap, r->bc, buf, (int)ov[0], (int)n, 0, NULL);
        if (ret == 1 && cap[0] == buf + ov[0]) return 1;
        return -1;
    }
    int k = 2 * r->ncap;
    for (int j = 0; j < k; j++) cap[j] = ov[j] == PCRE2_UNSET ? NULL : (uint8_t *)buf + ov[j];
    return 1;
}

/* The next match at or after `from` (UTF-16 units of the text in buf: bytes when ASCII), into
 * cap (r->nalloc pointers); false when there's none. Without want_groups only the whole match's
 * bounds need be right. */
static bool tv_re_find_in(tv_re *r, uint8_t **cap, const uint8_t *buf, tv_int n, tv_int from, bool ascii, int type, bool want_groups) {
    if (from < 0 || from > n) return false;
    if (r->simple && ascii) {
        for (tv_int i = from; i < n; i++) {
            if (r->simple_one >= 0) {
                const uint8_t *q = memchr(buf + i, r->simple_one, (size_t)(n - i));
                if (!q) return false;
                i = (tv_int)(q - buf);
            } else if (!r->set[buf[i]]) {
                continue;
            }
            tv_int j = i + 1;
            if (r->plus) while (j < n && r->set[buf[j]]) j++;
            cap[0] = (uint8_t *)buf + i;
            cap[1] = (uint8_t *)buf + j;
            return true;
        }
        return false;
    }
    if (r->lin && ascii) return tv_re_lin_find(r->lin, cap, buf, n, from);
    if (ascii && r->pstate != 2) {
        if (r->pstate == 0 && ++r->uses >= TV_PCRE_AFTER) tv_re_pcre_build(r);
        if (r->pstate == 1) {
            int found = tv_re_pcre_find(r, cap, buf, n, from, want_groups);
            if (found >= 0) return found;
        }
    }
    if (r->bc1 && ascii) {
        /* only where a match can start, each tried alone */
        bool ml = r->flags & LRE_FLAG_MULTILINE;
        for (tv_int i = from; i <= n; i++) {
            if (r->anchored) {
                if (r->afirst && i == 0) {
                    if (i < n && !r->first[buf[i]]) return false;
                }
                if (i > 0 && !(ml && (buf[i - 1] == '\n' || buf[i - 1] == '\r'))) {
                    if (!ml) return false;
                    const uint8_t *nl = memchr(buf + i, '\n', (size_t)(n - i)), *cr = memchr(buf + i, '\r', (size_t)(n - i));
                    if (!nl || (cr && cr < nl)) nl = cr;
                    if (!nl) return false;
                    i = (tv_int)(nl - buf) + 1;
                }
            } else if (r->lead >= 0) {
                const uint8_t *q = i < n ? memchr(buf + i, r->lead, (size_t)(n - i)) : NULL;
                if (!q) return false;
                i = (tv_int)(q - buf);
            } else {
                while (i < n && (buf[i] >= 128 || !r->first[buf[i]])) i++;
                if (i >= n) return false;
            }
            int ret = r->jit1 && type == 0 ? lre_exec_jit(r->jit1, cap, r->bc1, buf, (int)i, (int)n, NULL) : lre_exec(cap, r->bc1, buf, (int)i, (int)n, type, NULL);
            if (ret == LRE_RET_MEMORY_ERROR) tv_trap("out of memory in a regular expression", NULL);
            if (ret == 1) return true;
        }
        return false;
    }
    int ret = r->jit && type == 0 ? lre_exec_jit(r->jit, cap, r->bc, buf, (int)from, (int)n, NULL) : lre_exec(cap, r->bc, buf, (int)from, (int)n, type, NULL);
    if (ret == LRE_RET_MEMORY_ERROR) tv_trap("out of memory in a regular expression", NULL);
    return ret == 1;
}

static bool tv_re_find(tv_re *r, uint8_t **cap, const uint8_t *buf, tv_int n, tv_int from, bool ascii, int type) {
    return tv_re_find_in(r, cap, buf, n, from, ascii, type, true);
}

static tv_re *tv_res;
static tv_int tv_nres, tv_cap_res;
static tv_int *tv_re_index; /* open-addressed: id + 1, 0 = empty */
static tv_int tv_re_index_cap;

static void tv_re_index_put(tv_int id) {
    uint64_t h = tv_str_hash(tv_res[id].key);
    for (tv_int i = (tv_int)(h & (uint64_t)(tv_re_index_cap - 1));; i = (i + 1) & (tv_re_index_cap - 1)) {
        if (!tv_re_index[i]) {
            tv_re_index[i] = id + 1;
            return;
        }
    }
}

tv_int tv_native_reCompile(tv_str source, tv_str flags) {
    static const char order[] = "dgimsuvy"; /* RegExp.prototype.flags' order */
    int re_flags = 0;
    char seen[sizeof order] = {0};
    for (tv_int i = 0; i < tv_str_byte_len(flags); i++) {
        const char *at = strchr(order, flags.p->data[i]);
        if (!flags.p->data[i] || !at || seen[at - order]) {
            char msg[160];
            snprintf(msg, sizeof msg, "Invalid flags supplied to RegExp constructor '%.*s'", (int)tv_str_byte_len(flags), flags.p->data);
            tv_native_error(msg);
            return -1;
        }
        seen[at - order] = 1;
        switch (flags.p->data[i]) {
        case 'd': re_flags |= LRE_FLAG_INDICES; break;
        case 'g': re_flags |= LRE_FLAG_GLOBAL; break;
        case 'i': re_flags |= LRE_FLAG_IGNORECASE; break;
        case 'm': re_flags |= LRE_FLAG_MULTILINE; break;
        case 's': re_flags |= LRE_FLAG_DOTALL; break;
        case 'u': re_flags |= LRE_FLAG_UNICODE; break;
        case 'v': re_flags |= LRE_FLAG_UNICODE_SETS; break;
        case 'y': re_flags |= LRE_FLAG_STICKY; break;
        }
    }
    char flag_text[12];
    int nf = 0;
    for (size_t k = 0; k < sizeof order - 1; k++)
        if (seen[k]) flag_text[nf++] = order[k];
    flag_text[nf] = 0;
    /* already compiled? */
    tv_sb kb = {0};
    tv_sb_push(&kb, flag_text, (size_t)nf);
    tv_sb_push_char(&kb, '/');
    tv_sb_push_str(&kb, source);
    tv_str key = tv_str_from_sb(&kb);
    if (tv_re_index_cap) {
        uint64_t h = tv_str_hash(key);
        for (tv_int i = (tv_int)(h & (uint64_t)(tv_re_index_cap - 1)); tv_re_index[i]; i = (i + 1) & (tv_re_index_cap - 1)) {
            tv_int id = tv_re_index[i] - 1;
            if (tv_str_eq(tv_res[id].key, key)) {
                tv_str_release(key);
                return id;
            }
        }
    }
    char err[128];
    int len;
    /* (without `u`, the engine reads the source as CESU-8: a character past U+FFFF is its two
     * UTF-16 halves, each encoded as a 3-byte sequence) */
    const char *src = source.p->data;
    size_t src_len = (size_t)tv_str_byte_len(source);
    char *cesu = NULL;
    if (!(re_flags & (LRE_FLAG_UNICODE | LRE_FLAG_UNICODE_SETS))) {
        size_t wide = 0;
        for (size_t k = 0; k < src_len; k++) wide += (unsigned char)src[k] >= 0xF0;
        if (wide) {
            cesu = tv_alloc(src_len + 2 * wide + 1);
            size_t o = 0;
            for (size_t k = 0; k < src_len;) {
                unsigned char c = (unsigned char)src[k];
                if (c >= 0xF0 && k + 3 < src_len + 1) {
                    uint32_t cp = ((c & 7u) << 18) | (((unsigned char)src[k + 1] & 63u) << 12) | (((unsigned char)src[k + 2] & 63u) << 6) | ((unsigned char)src[k + 3] & 63u);
                    cp -= 0x10000;
                    uint32_t halves[2] = { 0xD800 + (cp >> 10), 0xDC00 + (cp & 0x3FF) };
                    for (int j = 0; j < 2; j++) {
                        cesu[o++] = (char)(0xE0 | (halves[j] >> 12));
                        cesu[o++] = (char)(0x80 | ((halves[j] >> 6) & 63));
                        cesu[o++] = (char)(0x80 | (halves[j] & 63));
                    }
                    k += 4;
                } else {
                    cesu[o++] = src[k++];
                }
            }
            cesu[o] = 0;
            src = cesu;
            src_len = o;
        }
    }
    uint8_t *bc = lre_compile(&len, err, sizeof err, src, src_len, re_flags, NULL);
    tv_free(cesu);
    if (!bc) {
        char msg[400];
        snprintf(msg, sizeof msg, "Invalid regular expression: /%.*s/%s: %s", (int)(tv_str_byte_len(source) > 200 ? 200 : tv_str_byte_len(source)), source.p->data, flag_text, err);
        tv_native_error(msg);
        tv_str_release(key);
        return -1;
    }
    if (tv_nres == tv_cap_res) {
        tv_cap_res = tv_cap_res ? tv_cap_res * 2 : 16;
        tv_res = tv_realloc(tv_res, (size_t)tv_cap_res * sizeof(tv_re));
    }
    tv_int id = tv_nres++;
    tv_re *r = &tv_res[id];
    r->bc = bc;
    r->key = key;
    r->ncap = lre_get_capture_count(bc);
    r->flags = re_flags;
    memcpy(r->flag_text, flag_text, (size_t)nf + 1);
    r->simple = r->ncap == 1 && tv_re_simple_parse(r, source.p->data, (size_t)tv_str_byte_len(source));
    r->bc1 = NULL;
    r->names = (tv_arr){NULL};
    r->anchored = false;
    r->afirst = false;
    r->lead = -1;
    r->lin = NULL;
    if (!r->simple) {
        tv_re_lin L;
        if (tv_re_lin_parse(r, source.p->data, (size_t)tv_str_byte_len(source), &L) && L.ngroups + 1 == r->ncap) {
            r->lin = tv_alloc(sizeof L);
            *r->lin = L;
        }
    }
    if (!r->simple && tv_re_first_parse(r, source.p->data, (size_t)tv_str_byte_len(source))) {
        int len1;
        char err1[64];
        r->bc1 = lre_compile(&len1, err1, sizeof err1, src, src_len, re_flags | LRE_FLAG_STICKY, NULL);
    }
    r->nalloc = 2 * r->ncap;
    if (lre_get_alloc_count(bc) > r->nalloc) r->nalloc = lre_get_alloc_count(bc);
    if (r->bc1 && lre_get_alloc_count(r->bc1) > r->nalloc) r->nalloc = lre_get_alloc_count(r->bc1);
    r->jit = r->jit1 = NULL;
    r->pc = r->md = NULL;
    r->uses = 0;
    r->pstate = 0;
    r->fixup = false;
    for (size_t k = 0; k < tv_nrejits; k++) {
        const tv_rejit *j = &tv_rejits[k];
        if (j->klen != (size_t)key.p->len || memcmp(j->key, key.p->data, j->klen)) continue;
        size_t bl = tv_bc_size(bc);
        if (j->bclen == bl && !memcmp(j->bc, bc, bl)) r->jit = j->fn;
        if (r->bc1 && j->fn1) {
            size_t bl1 = tv_bc_size(r->bc1);
            if (j->bc1len == bl1 && !memcmp(j->bc1, r->bc1, bl1)) r->jit1 = j->fn1;
        }
        break;
    }
    if (tv_nres * 2 > tv_re_index_cap) {
        tv_int cap = tv_re_index_cap ? tv_re_index_cap * 2 : 64;
        tv_free(tv_re_index);
        tv_re_index = tv_alloc((size_t)cap * sizeof(tv_int));
        memset(tv_re_index, 0, (size_t)cap * sizeof(tv_int));
        tv_re_index_cap = cap;
        for (tv_int i = 0; i < tv_nres; i++) tv_re_index_put(i);
    } else {
        tv_re_index_put(id);
    }
    return id;
}

tv_str tv_native_reFlags(tv_int h) {
    return tv_str_from(tv_res[h].flag_text, strlen(tv_res[h].flag_text));
}

/* The UTF-16 code units of a string that isn't ASCII (n of them; to free). */
static uint16_t *tv_utf16_of(tv_str s, tv_int *n) {
    const unsigned char *d = (const unsigned char *)s.p->data;
    size_t len = (size_t)tv_str_byte_len(s), k = 0;
    uint16_t *out = tv_alloc(len * sizeof(uint16_t) + 2);
    for (size_t i = 0; i < len;) {
        uint32_t c = d[i];
        size_t l = 1;
        if (c >= 0xF0 && i + 3 < len + 1) c = ((c & 7) << 18) | ((d[i + 1] & 63u) << 12) | ((d[i + 2] & 63u) << 6) | (d[i + 3] & 63u), l = 4;
        else if (c >= 0xE0) c = ((c & 15) << 12) | ((d[i + 1] & 63u) << 6) | (d[i + 2] & 63u), l = 3;
        else if (c >= 0xC0) c = ((c & 31) << 6) | (d[i + 1] & 63u), l = 2;
        if (c >= 0x10000) {
            c -= 0x10000;
            out[k++] = (uint16_t)(0xD800 + (c >> 10));
            out[k++] = (uint16_t)(0xDC00 + (c & 0x3FF));
        } else {
            out[k++] = (uint16_t)c;
        }
        i += l;
    }
    *n = (tv_int)k;
    return out;
}

/* The match at or after `from` (UTF-16 units): start and end of the whole match and of each
 * group (-1 where a group didn't take part), or an empty array when there's none. */
tv_arr tv_native_reExec(tv_int h, tv_str s, tv_int from) {
    tv_re *r = &tv_res[h];
    int ncap = r->ncap;
    uint8_t *on_stack[16];
    uint8_t **cap = r->nalloc <= 16 ? on_stack : tv_alloc((size_t)r->nalloc * sizeof(uint8_t *));
    bool ascii = tv_str_is_ascii(s);
    tv_int n;
    uint16_t *units = NULL;
    const uint8_t *buf;
    int type, shift;
    if (ascii) {
        buf = (const uint8_t *)s.p->data;
        n = tv_str_byte_len(s);
        type = 0;
        shift = 0;
    } else {
        units = tv_utf16_of(s, &n);
        buf = (const uint8_t *)units;
        type = 1; /* (UTF-16: lre_exec reads surrogate pairs itself for a `u` expression) */
        shift = 1;
    }
    tv_arr out;
    if (!tv_re_find(r, cap, buf, n, from, ascii, type)) {
        out = tv_arr_fresh();
    } else {
        out = tv_arr_with_capacity(&tv_type_int, 2 * ncap);
        tv_int *w = (tv_int *)tv_arr_data(out);
        for (int i = 0; i < 2 * ncap; i++) w[i] = cap[i] ? (tv_int)((cap[i] - buf) >> shift) : -1;
        out.p->len = 2 * ncap;
    }
    tv_free(units);
    if (cap != on_stack) tv_free(cap);
    return out;
}

/* Where the match at or after `from` ends (UTF-16 units), or -1 when there's none: RegExp.test's
 * question, answered with nothing allocated (for ASCII text and up to 8 groups). */
tv_int tv_native_reTest(tv_int h, tv_str s, tv_int from) {
    tv_re *r = &tv_res[h];
    /* (a single character set, `/[0-9.]/`, `/\s+/`, on ASCII text: a scan, nothing to set up) */
    if (r->simple && tv_str_is_ascii(s)) {
        const uint8_t *buf = (const uint8_t *)s.p->data;
        tv_int n = tv_str_byte_len(s);
        if (from < 0 || from > n) return -1;
        for (tv_int i = from; i < n; i++) {
            if (!r->set[buf[i]]) continue;
            tv_int j = i + 1;
            if (r->plus) while (j < n && r->set[buf[j]]) j++;
            return j;
        }
        return -1;
    }
    int ncap = r->ncap;
    uint8_t *on_stack[16];
    uint8_t **cap = r->nalloc <= 16 ? on_stack : tv_alloc((size_t)r->nalloc * sizeof(uint8_t *));
    bool ascii = tv_str_is_ascii(s);
    tv_int n, end = -1;
    uint16_t *units = NULL;
    const uint8_t *buf;
    int type = 0, shift = 0;
    if (ascii) {
        buf = (const uint8_t *)s.p->data;
        n = tv_str_byte_len(s);
    } else {
        units = tv_utf16_of(s, &n);
        buf = (const uint8_t *)units;
        type = 1;
        shift = 1;
    }
    if (tv_re_find_in(r, cap, buf, n, from, ascii, type, false)) end = (tv_int)((cap[1] - buf) >> shift);
    tv_free(units);
    if (cap != on_stack) tv_free(cap);
    return end;
}

/* A string's text for matching: its bytes when ASCII, else its UTF-16 units (to free). */
typedef struct { const uint8_t *buf; uint16_t *units; tv_int n; bool ascii; int type, shift; } tv_re_text;
static tv_re_text tv_re_text_of(tv_str s) {
    tv_re_text t = {0};
    t.ascii = tv_str_is_ascii(s);
    if (t.ascii) {
        t.buf = (const uint8_t *)s.p->data;
        t.n = tv_str_byte_len(s);
    } else {
        t.units = tv_utf16_of(s, &t.n);
        t.buf = (const uint8_t *)t.units;
        t.type = 1;
        t.shift = 1;
    }
    return t;
}
static tv_int tv_re_pos(const tv_re_text *t, const uint8_t *p) { return p ? (tv_int)((p - t->buf) >> t->shift) : -1; }
static tv_str tv_re_slice(tv_str s, const tv_re_text *t, tv_int a, tv_int b) {
    return t->ascii ? tv_str_from(s.p->data + a, (size_t)(b - a)) : tv_str_slice(s, a, b, true, NULL);
}
/* Where the search goes on after a match (past an empty one: the next character). */
static tv_int tv_re_next(tv_re *r, const tv_re_text *t, tv_int p0, tv_int p1) {
    if (p1 > p0) return p1;
    if ((r->flags & (LRE_FLAG_UNICODE | LRE_FLAG_UNICODE_SETS)) && !t->ascii && p1 + 1 < t->n && t->units[p1] >= 0xD800 && t->units[p1] <= 0xDBFF) return p1 + 2;
    return p1 + 1;
}

/* s.split(re, max), as the prelude's __reSplit did with reExec (the groups of each match between
 * the parts; groups that didn't take part are left out). */
tv_arr tv_native_reSplit(tv_int h, tv_str s, tv_int max) {
    tv_re *r = &tv_res[h];
    tv_arr out = tv_arr_fresh();
    if (max == 0) return out;
    uint8_t **cap = tv_alloc((size_t)r->nalloc * sizeof(uint8_t *));
    tv_re_text t = tv_re_text_of(s);
    if (t.n == 0) {
        if (!tv_re_find(r, cap, t.buf, 0, 0, t.ascii, t.type)) { tv_str_retain(s); tv_arr_push(&out, &tv_type_str, &s); }
    } else {
        tv_int last = 0, from = 0;
        while (from < t.n) {
            if (!tv_re_find(r, cap, t.buf, t.n, from, t.ascii, t.type)) break;
            tv_int p0 = tv_re_pos(&t, cap[0]), p1 = tv_re_pos(&t, cap[1]);
            if (p0 >= t.n) break;
            if (p1 == p0 && p0 == last) { from = tv_re_next(r, &t, p0, p1); continue; }
            tv_str part = tv_re_slice(s, &t, last, p0);
            tv_arr_push(&out, &tv_type_str, &part);
            if (tv_arr_len(out) == max) goto done;
            for (int k = 1; k < r->ncap; k++) {
                tv_int a = tv_re_pos(&t, cap[2 * k]), b = tv_re_pos(&t, cap[2 * k + 1]);
                if (a < 0) continue;
                tv_str g = tv_re_slice(s, &t, a, b);
                tv_arr_push(&out, &tv_type_str, &g);
                if (tv_arr_len(out) == max) goto done;
            }
            last = p1;
            from = tv_re_next(r, &t, p0, p1);
        }
        tv_str rest = tv_re_slice(s, &t, last, t.n);
        tv_arr_push(&out, &tv_type_str, &rest);
    }
done:
    tv_free(t.units);
    tv_free(cap);
    return out;
}

/* s.replace(re, rep) for a replacement without `$` patterns and an expression that isn't sticky:
 * every match with the `g` flag, else the first. */
/* reExec, into `p` when nothing else holds it and it has room (a loop over the matches, its
 * array given back each time, makes one array for them all); else a new array. */
tv_arr tv_native_reExecInto(tv_int h, tv_str s, tv_int from, tv_arr p) {
    tv_re *r = &tv_res[h];
    int ncap = r->ncap;
    if (!(p.p->rc == 1 && !p.p->ext && p.p->cap >= 2 * ncap)) return tv_native_reExec(h, s, from);
    uint8_t *on_stack[16];
    uint8_t **cap = r->nalloc <= 16 ? on_stack : tv_alloc((size_t)r->nalloc * sizeof(uint8_t *));
    tv_re_text t = tv_re_text_of(s);
    tv_arr out;
    if (tv_re_find(r, cap, t.buf, t.n, from, t.ascii, t.type)) {
        tv_int *w = (tv_int *)tv_arr_data(p);
        for (int i = 0; i < 2 * ncap; i++) w[i] = tv_re_pos(&t, cap[i]);
        p.p->len = 2 * ncap;
        tv_arr_retain(p);
        out = p;
    } else {
        out = tv_arr_fresh();
    }
    tv_free(t.units);
    if (cap != on_stack) tv_free(cap);
    return out;
}

/* The matches of h in s one by one (`s.replace(re, m => ...)` compiled to a loop): the text made
 * once (UTF-16 for a non-ASCII string), each next match's start and end into pos. */
void tv_re_iter_init(tv_re_iter *it, tv_int h, tv_str s) {
    tv_re *r = &tv_res[h];
    tv_re_text t = tv_re_text_of(s);
    it->h = h;
    it->buf = t.buf;
    it->units = t.units;
    it->n = t.n;
    it->ascii = t.ascii;
    it->type = t.type;
    it->shift = t.shift;
    it->from = 0;
    it->cap = r->nalloc <= 16 ? it->on_stack : tv_alloc((size_t)r->nalloc * sizeof(uint8_t *));
}

bool tv_re_iter_next(tv_re_iter *it, tv_int pos[2]) {
    tv_re *r = &tv_res[it->h];
    if (it->from > it->n || !tv_re_find(r, it->cap, it->buf, it->n, it->from, it->ascii, it->type)) return false;
    tv_re_text t = {.buf = it->buf, .units = it->units, .n = it->n, .ascii = it->ascii, .type = it->type, .shift = it->shift};
    pos[0] = tv_re_pos(&t, it->cap[0]);
    pos[1] = tv_re_pos(&t, it->cap[1]);
    it->from = tv_re_next(r, &t, pos[0], pos[1]);
    return true;
}

tv_str tv_re_iter_group(const tv_re_iter *it, tv_str s, int k) {
    tv_re *r = &tv_res[it->h];
    tv_re_text t = {.buf = it->buf, .units = it->units, .n = it->n, .ascii = it->ascii, .type = it->type, .shift = it->shift};
    /* (past the groups: the match's offset, as text, then the input, as JavaScript passes them) */
    if (k == r->ncap) return tv_str_from_int(tv_re_pos(&t, it->cap[0]));
    if (k == r->ncap + 1) { tv_str_retain(s); return s; }
    if (k >= r->ncap || !it->cap[2 * k]) return TV_EMPTY_STR;
    return tv_re_slice(s, &t, tv_re_pos(&t, it->cap[2 * k]), tv_re_pos(&t, it->cap[2 * k + 1]));
}

void tv_re_iter_done(tv_re_iter *it) {
    tv_free(it->units);
    if (it->cap != it->on_stack) tv_free(it->cap);
}

/* `s.match(re)` of a global re: every match's text (none: an empty array), in one pass. */
tv_arr tv_native_reMatchAll(tv_int h, tv_str s) {
    tv_re_iter it;
    tv_re_iter_init(&it, h, s);
    /* (gathered first, then one array of exactly their number: no regrowing) */
    tv_str on_stack[64], *found = on_stack;
    tv_int n = 0, cap = 64, pos[2];
    while (tv_re_iter_next(&it, pos)) {
        if (n == cap) {
            tv_str *more = tv_alloc((size_t)cap * 2 * sizeof(tv_str));
            memcpy(more, found, (size_t)n * sizeof(tv_str));
            if (found != on_stack) tv_free(found);
            found = more;
            cap *= 2;
        }
        found[n++] = tv_re_iter_group(&it, s, 0);
    }
    tv_re_iter_done(&it);
    tv_arr out = tv_arr_new_len(sizeof(tv_str), n);
    if (n) memcpy(tv_arr_data(out), found, (size_t)n * sizeof(tv_str));
    if (found != on_stack) tv_free(found);
    return out;
}

/* Units a..b of s (UTF-16 positions) onto sb. */
static void tv_re_push_units(tv_sb *sb, tv_str s, const tv_re_text *t, tv_int a, tv_int b) {
    if (b <= a) return;
    if (t->ascii) {
        tv_sb_push(sb, s.p->data + a, (size_t)(b - a));
        return;
    }
    tv_str piece = tv_str_slice(s, a, b, true, NULL);
    tv_sb_push_str(sb, piece);
    tv_str_release(piece);
}

/* A replacement's `$` patterns for one match ($$, $&, $`, $', $1..$99; `$<name>` is the
 * prelude's), as String.prototype.replace expands them. */
static void tv_re_subst(tv_sb *sb, tv_str s, const tv_re_text *t, uint8_t **cap, int ncap, tv_str rep) {
    const char *d = rep.p->data;
    size_t n = (size_t)tv_str_byte_len(rep), i = 0, run = 0;
    while (i < n) {
        if (d[i] != '$' || i + 1 >= n) {
            i++;
            continue;
        }
        char c = d[i + 1];
        tv_int p0 = tv_re_pos(t, cap[0]), p1 = tv_re_pos(t, cap[1]);
        size_t used = 2;
        if (c == '$') {
            tv_sb_push(sb, d + run, i - run);
            tv_sb_push(sb, "$", 1);
        } else if (c == '&') {
            tv_sb_push(sb, d + run, i - run);
            tv_re_push_units(sb, s, t, p0, p1);
        } else if (c == '`') {
            tv_sb_push(sb, d + run, i - run);
            tv_re_push_units(sb, s, t, 0, p0);
        } else if (c == '\'') {
            tv_sb_push(sb, d + run, i - run);
            tv_re_push_units(sb, s, t, p1, t->n);
        } else if (c >= '0' && c <= '9') {
            int k = c - '0';
            if (i + 2 < n && d[i + 2] >= '0' && d[i + 2] <= '9' && k * 10 + (d[i + 2] - '0') < ncap && k * 10 + (d[i + 2] - '0') >= 1) {
                k = k * 10 + (d[i + 2] - '0');
                used = 3;
            }
            if (k < 1 || k >= ncap) {
                i++;
                continue;
            }
            tv_sb_push(sb, d + run, i - run);
            if (cap[2 * k]) tv_re_push_units(sb, s, t, tv_re_pos(t, cap[2 * k]), tv_re_pos(t, cap[2 * k + 1]));
        } else {
            i++;
            continue;
        }
        i += used;
        run = i;
    }
    tv_sb_push(sb, d + run, n - run);
}

tv_str tv_native_reReplace(tv_int h, tv_str s, tv_str rep) {
    tv_re *r = &tv_res[h];
    bool global = r->flags & LRE_FLAG_GLOBAL;
    bool dollar = memchr(rep.p->data, '$', (size_t)tv_str_byte_len(rep)) != NULL;
    uint8_t *on_stack[16];
    uint8_t **cap = r->nalloc <= 16 ? on_stack : tv_alloc((size_t)r->nalloc * sizeof(uint8_t *));
    tv_re_text t = tv_re_text_of(s);
    /* (ASCII text and a plain replacement: the matches found first, then the result written at
     * its size, no builder to grow and copy) */
    if (t.ascii && !dollar) {
        enum { MAXM = 32 };
        tv_int pos[2 * MAXM], m = 0, from = 0;
        bool more = false;
        while (from <= t.n && tv_re_find(r, cap, t.buf, t.n, from, true, t.type)) {
            if (m == MAXM) { more = true; break; }
            tv_int p0 = (tv_int)(cap[0] - t.buf), p1 = (tv_int)(cap[1] - t.buf);
            pos[2 * m] = p0, pos[2 * m + 1] = p1, m++;
            if (!global) break;
            from = tv_re_next(r, &t, p0, p1);
        }
        if (!more) {
            if (!m) {
                if (cap != on_stack) tv_free(cap);
                tv_str_retain(s);
                return s;
            }
            size_t rl = (size_t)tv_str_byte_len(rep), len = (size_t)t.n;
            for (tv_int i = 0; i < m; i++) len = len - (size_t)(pos[2 * i + 1] - pos[2 * i]) + rl;
            if (len <= 1) goto built; /* (empty or one byte: the shared strings, by the builder) */
            if (cap != on_stack) tv_free(cap);
            char *o;
            tv_str out = tv_str_alloc(len, &o);
            tv_int last = 0;
            for (tv_int i = 0; i < m; i++) {
                memcpy(o, t.buf + last, (size_t)(pos[2 * i] - last)); o += pos[2 * i] - last;
                memcpy(o, rep.p->data, rl); o += rl;
                last = pos[2 * i + 1];
            }
            memcpy(o, t.buf + last, (size_t)(t.n - last));
            /* (ASCII in, ASCII out when the replacement is) */
            if (tv_str_is_ascii(rep) && out.p->rc > 0) out.p->rc |= TV_STR_ASCII;
            return out;
        }
    }
built:;
    tv_sb sb = {0};
    tv_int last = 0, from = 0;
    bool any = false;
    while (from <= t.n && tv_re_find(r, cap, t.buf, t.n, from, t.ascii, t.type)) {
        tv_int p0 = tv_re_pos(&t, cap[0]), p1 = tv_re_pos(&t, cap[1]);
        any = true;
        tv_re_push_units(&sb, s, &t, last, p0);
        if (dollar) tv_re_subst(&sb, s, &t, cap, r->ncap, rep);
        else tv_sb_push_str(&sb, rep);
        last = p1;
        if (!global) break;
        from = tv_re_next(r, &t, p0, p1);
    }
    tv_str out;
    if (!any) {
        tv_str_retain(s);
        out = s;
        tv_sb_free(&sb);
    } else {
        tv_re_push_units(&sb, s, &t, last, t.n);
        out = tv_str_from_sb(&sb);
    }
    tv_free(t.units);
    if (cap != on_stack) tv_free(cap);
    return out;
}

tv_arr tv_native_reNames(tv_int h) {
    tv_re *r = &tv_res[h];
    if (r->names.p) {
        tv_arr_retain(r->names);
        return r->names;
    }
    tv_arr out = tv_arr_fresh();
    const char *names = (r->flags & LRE_FLAG_NAMED_GROUPS) || lre_get_flags(r->bc) & LRE_FLAG_NAMED_GROUPS ? lre_get_groupnames(r->bc) : NULL;
    for (int i = 0; i < r->ncap; i++) {
        tv_str name = TV_EMPTY_STR;
        if (i > 0 && names) {
            size_t len = strlen(names);
            name = tv_str_from(names, len);
            names += len + 1 + LRE_GROUP_NAME_TRAILER_LEN - 1;
        }
        tv_arr_push(&out, &tv_type_str, &name);
    }
    r->names = out;
    tv_arr_retain(out);
    return out;
}
