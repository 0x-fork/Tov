// The Benchmarks Game fasta (single-threaded): an ALU repeat and two sequences drawn from
// nucleotide tables with the LCG, written as 60-column lines. Each line goes to a sink that
// counts lines and bytes and checksums the bytes instead of writing them to stdout.
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define WIDTH 60
#define IM 139968
#define IA 3877
#define IC 29573

static const char ALU[] =
    "GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGG"
    "GAGGCCGAGGCGGGCGGATCACCTGAGGTCAGGAGTTCGAGA"
    "CCAGCCTGGCCAACATGGTGAAACCCCGTCTCTACTAAAAAT"
    "ACAAAAATTAGCCGGGCGTGGTGGCGCGCGCCTGTAATCCCA"
    "GCTACTCGGGAGGCTGAGGCAGGAGAATCGCTTGAACCCGGG"
    "AGGCGGAGGTTGCAGTGAGCCGAGATCGCGCCACTGCACTCC"
    "AGCCTGGGCGACAGAGCGAGACTCCGTCTCAAAAA";

typedef struct {
    int64_t lines;
    int64_t bytes;
    uint32_t hash;
} Sink;

// One line: its bytes, then '\n'.
static void write_line(Sink *out, const char *buf, size_t len) {
    uint32_t h = out->hash;
    for (size_t i = 0; i < len; i++) {
        h = (h * 31 + (unsigned char)buf[i]) & 0xffffff;
    }
    out->hash = (h * 31 + '\n') & 0xffffff;
    out->bytes += (int64_t)len + 1;
    out->lines++;
}

static uint32_t seed = 42;

static double random_next(void) {
    seed = (seed * IA + IC) % IM;
    return (double)seed / IM;
}

static void repeat_fasta(Sink *out, const char *header, const char *src, int64_t n) {
    write_line(out, header, strlen(header));
    size_t len = strlen(src);
    char s[sizeof ALU + WIDTH];
    memcpy(s, src, len);
    memcpy(s + len, src, WIDTH);
    size_t pos = 0;
    while (n > 0) {
        size_t w = n < WIDTH ? (size_t)n : WIDTH;
        write_line(out, s + pos, w);
        pos += w;
        if (pos >= len) pos -= len;
        n -= (int64_t)w;
    }
}

static void random_fasta(Sink *out, const char *header, const char *chars, const double *probs, int count, int64_t n) {
    write_line(out, header, strlen(header));
    double cum[16];
    double p = 0.0;
    for (int i = 0; i < count; i++) {
        p += probs[i];
        cum[i] = p;
    }
    const int last = count - 1;
    char line[WIDTH];
    while (n > 0) {
        size_t w = n < WIDTH ? (size_t)n : WIDTH;
        for (size_t j = 0; j < w; j++) {
            const double r = random_next();
            int i = 0;
            while (i < last && cum[i] <= r) i++;
            line[j] = chars[i];
        }
        write_line(out, line, w);
        n -= (int64_t)w;
    }
}

int main(void) {
    const int64_t n = 10000000;
    static const double iub[] = {0.27, 0.12, 0.12, 0.27, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02};
    static const double homo_sapiens[] = {0.3029549426680, 0.1979883004921, 0.1975473066391, 0.3015094502008};
    Sink out = {0, 0, 0};
    repeat_fasta(&out, ">ONE Homo sapiens alu", ALU, 2 * n);
    random_fasta(&out, ">TWO IUB ambiguity codes", "acgtBDHKMNRSVWY", iub, 15, 3 * n);
    random_fasta(&out, ">THREE Homo sapiens frequency", "acgt", homo_sapiens, 4, 5 * n);
    printf("%lld %lld %u\n", (long long)out.lines, (long long)out.bytes, out.hash);
    return 0;
}
