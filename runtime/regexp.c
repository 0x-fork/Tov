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
} tv_re;

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
    uint8_t *bc = lre_compile(&len, err, sizeof err, source.p->data, (size_t)tv_str_byte_len(source), re_flags, NULL);
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
        type = (r->flags & (LRE_FLAG_UNICODE | LRE_FLAG_UNICODE_SETS)) ? 2 : 1;
        shift = 1;
    }
    tv_arr out = tv_arr_fresh();
    if (from >= 0 && from <= n) {
        int ret = lre_exec(cap, r->bc, buf, (int)from, (int)n, type, NULL);
        if (ret == LRE_RET_MEMORY_ERROR) tv_trap("out of memory in a regular expression", NULL);
        if (ret == 1) {
            tv_int *w = (tv_int *)tv_arr_reserve_tail(&out, &tv_type_int, 2 * ncap);
            for (int i = 0; i < 2 * ncap; i++) w[i] = cap[i] ? (tv_int)((cap[i] - buf) >> shift) : -1;
            out.p->len = 2 * ncap;
        }
    }
    tv_free(units);
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
