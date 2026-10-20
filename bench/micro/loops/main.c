// The "1 billion nested loop iterations" program: a[i] += j % u over 10,000 x 100,000.
// u and r come from the argument count at runtime, so they can't be constant-folded.
#include <stdint.h>
#include <stdio.h>

int main(int argc, char **argv) {
    (void)argv;
    int32_t u = 40 + (argc - 1);
    int64_t state = u;
    for (int k = 0; k < 3; k++) {
        state = (state * 16807) % 2147483647;
    }
    int32_t r = (int32_t)(state % 10000);
    static int32_t a[10000];
    for (int32_t i = 0; i < 10000; i++) {
        for (int32_t j = 0; j < 100000; j++) {
            a[i] += j % u;
        }
        a[i] += r;
    }
    printf("%d\n", a[r]);
    return 0;
}
