// Levenshtein distance (two-row DP, byte comparisons) between every pair of N words
// generated with a Park-Miller LCG. Prints the minimum distance and the sum of all distances.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static size_t levenshtein(const char *s, const char *t) {
    size_t sl = strlen(s), tl = strlen(t);
    if (sl > tl) {
        const char *x = s; s = t; t = x;
        size_t y = sl; sl = tl; tl = y;
    }
    size_t m = sl, n = tl;
    size_t *prev = malloc((m + 1) * sizeof(size_t));
    size_t *curr = malloc((m + 1) * sizeof(size_t));
    if (!prev || !curr) abort();
    for (size_t i = 0; i <= m; i++) prev[i] = i;
    for (size_t j = 1; j <= n; j++) {
        curr[0] = j;
        char tj = t[j - 1];
        for (size_t i = 1; i <= m; i++) {
            size_t cost = s[i - 1] == tj ? 0 : 1;
            size_t del = prev[i] + 1, ins = curr[i - 1] + 1, sub = prev[i - 1] + cost;
            size_t best = del < ins ? del : ins;
            curr[i] = best < sub ? best : sub;
        }
        size_t *tmp = prev; prev = curr; curr = tmp;
    }
    size_t d = prev[m];
    free(prev);
    free(curr);
    return d;
}

int main(void) {
    const char *alphabet = "abcdefgh";
    const size_t count = 800;
    int64_t state = 42;
    char **words = malloc(count * sizeof(char *));
    if (!words) abort();
    for (size_t w = 0; w < count; w++) {
        state = (state * 16807) % 2147483647;
        size_t len = 20 + (size_t)(state % 41);
        char *s = malloc(len + 1);
        if (!s) abort();
        for (size_t k = 0; k < len; k++) {
            state = (state * 16807) % 2147483647;
            s[k] = alphabet[state % 8];
        }
        s[len] = '\0';
        words[w] = s;
    }
    size_t min_dist = SIZE_MAX, total = 0;
    for (size_t i = 0; i < count; i++) {
        for (size_t j = i + 1; j < count; j++) {
            size_t d = levenshtein(words[i], words[j]);
            if (d < min_dist) min_dist = d;
            total += d;
        }
    }
    printf("%zu %zu\n", min_dist, total);
    for (size_t w = 0; w < count; w++) free(words[w]);
    free(words);
    return 0;
}
