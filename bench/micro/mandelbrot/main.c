// The Benchmarks Game mandelbrot (single-threaded): an N×N bitmap of the Mandelbrot set,
// 8 pixels to a byte. Instead of writing the PBM it prints the size and a checksum of the bytes.
#include <stdint.h>
#include <stdio.h>

// No a*b+c fusing, as in Rust and JS (Apple clang fuses by default): pixels on the set's edge
// would otherwise come out differently.
#pragma STDC FP_CONTRACT OFF

int main(void) {
    const int n = 4000;
    const int iter = 50;
    const double limit = 4.0;
    int64_t bytes = 0;
    int64_t checksum = 0;
    for (int y = 0; y < n; y++) {
        const double ci = 2.0 * y / n - 1.0;
        int byte = 0;
        int bit = 0;
        for (int x = 0; x < n; x++) {
            const double cr = 2.0 * x / n - 1.5;
            double zr = 0.0, zi = 0.0, tr = 0.0, ti = 0.0;
            for (int i = 0; i < iter && tr + ti <= limit; i++) {
                zi = 2.0 * zr * zi + ci;
                zr = tr - ti + cr;
                tr = zr * zr;
                ti = zi * zi;
            }
            byte <<= 1;
            if (tr + ti <= limit) byte |= 1;
            bit++;
            if (bit == 8 || x == n - 1) {
                byte <<= 8 - bit;
                checksum = (checksum * 31 + byte) & 0xffffff;
                bytes++;
                byte = 0;
                bit = 0;
            }
        }
    }
    printf("%d %lld %lld\n", n, (long long)bytes, (long long)checksum);
    return 0;
}
