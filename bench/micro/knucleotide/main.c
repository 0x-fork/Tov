// The Benchmarks Game k-nucleotide (single-threaded), on a DNA sequence generated in the program
// with fasta's LCG and Homo sapiens frequencies: count every k-mer for k = 1, 2 (printed as sorted
// percentages) and for the lengths of five given k-mers (printed as counts), with a string-keyed
// hash table. The table is a plain open-addressing one (FNV-1a, linear probing, cached hashes);
// its keys point into the sequence.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define IM 139968
#define IA 3877
#define IC 29573

typedef struct {
    const char *key; // NULL = empty slot; `len` bytes of the sequence
    uint64_t hash;
    int64_t count;
} Entry;

typedef struct {
    Entry *entries;
    size_t cap; // power of two
    size_t len;
    size_t klen; // every key has this length
} Table;

static uint64_t hash(const char *s, size_t n) {
    uint64_t h = 14695981039346656037ULL;
    for (size_t i = 0; i < n; i++) {
        h ^= (unsigned char)s[i];
        h *= 1099511628211ULL;
    }
    return h;
}

static Entry *find_slot(Entry *entries, size_t cap, size_t klen, const char *key, uint64_t h) {
    size_t i = h & (cap - 1);
    while (entries[i].key && (entries[i].hash != h || memcmp(entries[i].key, key, klen) != 0)) {
        i = (i + 1) & (cap - 1);
    }
    return &entries[i];
}

static void table_grow(Table *t) {
    size_t cap = t->cap ? t->cap * 2 : 16;
    Entry *entries = calloc(cap, sizeof(Entry));
    if (!entries) abort();
    for (size_t i = 0; i < t->cap; i++) {
        if (t->entries[i].key) {
            *find_slot(entries, cap, t->klen, t->entries[i].key, t->entries[i].hash) = t->entries[i];
        }
    }
    free(t->entries);
    t->entries = entries;
    t->cap = cap;
}

static void table_increment(Table *t, const char *key) {
    if ((t->len + 1) * 4 > t->cap * 3) table_grow(t);
    uint64_t h = hash(key, t->klen);
    Entry *e = find_slot(t->entries, t->cap, t->klen, key, h);
    if (!e->key) {
        e->key = key;
        e->hash = h;
        t->len++;
    }
    e->count++;
}

static int64_t table_get(const Table *t, const char *key) {
    Entry *e = find_slot(t->entries, t->cap, t->klen, key, hash(key, t->klen));
    return e->key ? e->count : 0;
}

static Table frequencies(const char *seq, size_t n, size_t k) {
    Table t = {NULL, 0, 0, k};
    for (size_t i = 0; i + k <= n; i++) {
        table_increment(&t, seq + i);
    }
    return t;
}

static size_t sort_klen;

static int by_count_then_key(const void *pa, const void *pb) {
    const Entry *a = pa, *b = pb;
    if (a->count != b->count) return a->count > b->count ? -1 : 1;
    return memcmp(a->key, b->key, sort_klen);
}

static void print_frequencies(const char *seq, size_t n, size_t k) {
    Table t = frequencies(seq, n, k);
    Entry *list = malloc(t.len * sizeof(Entry));
    if (!list) abort();
    size_t m = 0;
    for (size_t i = 0; i < t.cap; i++) {
        if (t.entries[i].key) list[m++] = t.entries[i];
    }
    sort_klen = k;
    qsort(list, m, sizeof(Entry), by_count_then_key);
    const int64_t total = (int64_t)(n - k + 1);
    for (size_t i = 0; i < m; i++) {
        printf("%.*s %.3f\n", (int)k, list[i].key, 100.0 * list[i].count / total);
    }
    printf("\n");
    free(list);
    free(t.entries);
}

static void print_count(const char *seq, size_t n, const char *fragment) {
    size_t k = strlen(fragment);
    Table t = frequencies(seq, n, k);
    printf("%lld\t%s\n", (long long)table_get(&t, fragment), fragment);
    free(t.entries);
}

int main(void) {
    const size_t n = 5000000;
    static const char chars[] = "ACGT";
    static const double probs[] = {0.3029549426680, 0.1979883004921, 0.1975473066391, 0.3015094502008};
    double cum[4];
    double p = 0.0;
    for (int i = 0; i < 4; i++) {
        p += probs[i];
        cum[i] = p;
    }
    char *seq = malloc(n);
    if (!seq) abort();
    uint32_t seed = 42;
    for (size_t j = 0; j < n; j++) {
        seed = (seed * IA + IC) % IM;
        const double r = (double)seed / IM;
        int i = 0;
        while (i < 3 && cum[i] <= r) i++;
        seq[j] = chars[i];
    }

    print_frequencies(seq, n, 1);
    print_frequencies(seq, n, 2);
    static const char *const fragments[] = {"GGT", "GGTA", "GGTATT", "GGTATTTTAATT", "GGTATTTTAATTTATAGT"};
    for (int i = 0; i < 5; i++) print_count(seq, n, fragments[i]);
    free(seq);
    return 0;
}
