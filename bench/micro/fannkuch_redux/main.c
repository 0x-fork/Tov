// The Benchmarks Game fannkuch-redux (single-threaded): count pancake flips over every permutation.
#include <stdio.h>

#define MAX_N 16

static void fannkuch(int n, int *checksum_out, int *max_flips_out) {
    int perm[MAX_N], perm1[MAX_N], count[MAX_N];
    int max_flips = 0, checksum = 0, perm_count = 0;
    for (int i = 0; i < n; i++) perm1[i] = i;
    int r = n;
    for (;;) {
        while (r != 1) {
            count[r - 1] = r;
            r--;
        }
        for (int i = 0; i < n; i++) perm[i] = perm1[i];
        int flips = 0;
        int k;
        while ((k = perm[0]) != 0) {
            for (int i = 0, j = k; i < j; i++, j--) {
                int t = perm[i];
                perm[i] = perm[j];
                perm[j] = t;
            }
            flips++;
        }
        if (flips > max_flips) max_flips = flips;
        checksum += perm_count % 2 == 0 ? flips : -flips;
        for (;;) {
            if (r == n) {
                *checksum_out = checksum;
                *max_flips_out = max_flips;
                return;
            }
            int perm0 = perm1[0];
            for (int i = 0; i < r; i++) perm1[i] = perm1[i + 1];
            perm1[r] = perm0;
            count[r]--;
            if (count[r] > 0) break;
            r++;
        }
        perm_count++;
    }
}

int main(void) {
    const int n = 11;
    int checksum, max_flips;
    fannkuch(n, &checksum, &max_flips);
    printf("%d\nPfannkuchen(%d) = %d\n", checksum, n, max_flips);
    return 0;
}
