/* Regular expressions: JavaScript's, compiled and run by QuickJS's libregexp (vendor/quickjs-regexp).
 * Linked only into programs that use them. The prelude's RegExp class and string methods are
 * written in Tov over these natives; positions are UTF-16 code units, as in JavaScript (an ASCII
 * string is matched as its bytes, anything else as UTF-16). */
#include "tov.h"
#include "libregexp.h"
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
    int flags;
    char flag_text[12];
    /* One character of a set, or a run of them (`\s+`, `[^a-z]`, `,`): on ASCII text, found by
     * a table instead of the engine. */
    bool simple, plus;
    bool set[128];
} tv_re;

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
        if (r->flags & LRE_FLAG_UNICODE_SETS) return false; /* (`v`: classes have set operations) */
        i = 1;
        bool neg = i < n && src[i] == '^';
        if (neg) i++;
        bool pos[128] = {0}, any = false;
        while (i < n && src[i] != ']') {
            int lo;
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
    return true;
}

/* The next match at or after `from` (UTF-16 units of the text in buf: bytes when ASCII), into
 * cap (2 * ncap pointers); false when there's none. */
static bool tv_re_find(const tv_re *r, uint8_t **cap, const uint8_t *buf, tv_int n, tv_int from, bool ascii, int type) {
    if (from < 0 || from > n) return false;
    if (r->simple && ascii) {
        for (tv_int i = from; i < n; i++) {
            if (!r->set[buf[i]]) continue;
            tv_int j = i + 1;
            if (r->plus) while (j < n && r->set[buf[j]]) j++;
            cap[0] = (uint8_t *)buf + i;
            cap[1] = (uint8_t *)buf + j;
            return true;
        }
        return false;
    }
    int ret = lre_exec(cap, r->bc, buf, (int)from, (int)n, type, NULL);
    if (ret == LRE_RET_MEMORY_ERROR) tv_trap("out of memory in a regular expression", NULL);
    return ret == 1;
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
    uint8_t **cap = tv_alloc((size_t)(2 * ncap) * sizeof(uint8_t *) + 1);
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
    tv_arr out = tv_arr_fresh();
    if (tv_re_find(r, cap, buf, n, from, ascii, type)) {
        tv_int *w = (tv_int *)tv_arr_reserve_tail(&out, &tv_type_int, 2 * ncap);
        for (int i = 0; i < 2 * ncap; i++) w[i] = cap[i] ? (tv_int)((cap[i] - buf) >> shift) : -1;
        out.p->len = 2 * ncap;
    }
    tv_free(units);
    tv_free(cap);
    return out;
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
static tv_int tv_re_next(const tv_re *r, const tv_re_text *t, tv_int p0, tv_int p1) {
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
    uint8_t **cap = tv_alloc((size_t)(2 * r->ncap) * sizeof(uint8_t *) + 1);
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
tv_str tv_native_reReplace(tv_int h, tv_str s, tv_str rep) {
    tv_re *r = &tv_res[h];
    bool global = r->flags & LRE_FLAG_GLOBAL;
    uint8_t **cap = tv_alloc((size_t)(2 * r->ncap) * sizeof(uint8_t *) + 1);
    tv_re_text t = tv_re_text_of(s);
    tv_sb sb = {0};
    tv_int last = 0, from = 0;
    bool any = false;
    while (from <= t.n && tv_re_find(r, cap, t.buf, t.n, from, t.ascii, t.type)) {
        tv_int p0 = tv_re_pos(&t, cap[0]), p1 = tv_re_pos(&t, cap[1]);
        any = true;
        if (t.ascii) tv_sb_push(&sb, s.p->data + last, (size_t)(p0 - last));
        else { tv_str piece = tv_str_slice(s, last, p0, true, NULL); tv_sb_push_str(&sb, piece); tv_str_release(piece); }
        tv_sb_push_str(&sb, rep);
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
        if (t.ascii) tv_sb_push(&sb, s.p->data + last, (size_t)(t.n - last));
        else { tv_str piece = tv_str_slice(s, last, t.n, true, NULL); tv_sb_push_str(&sb, piece); tv_str_release(piece); }
        out = tv_str_from_sb(&sb);
    }
    tv_free(t.units);
    tv_free(cap);
    return out;
}

/* Each group's name ("" for the whole match and unnamed groups). */
tv_arr tv_native_reNames(tv_int h) {
    tv_re *r = &tv_res[h];
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
    return out;
}
