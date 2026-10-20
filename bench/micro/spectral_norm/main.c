// The Benchmarks Game spectral-norm (single-threaded): power iteration on an infinite matrix.
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

// The denominator of A[i][j] = 1 / ((i + j) * (i + j + 1) / 2 + i + 1).
static double a(int i, int j) {
    return (double)((i + j) * (i + j + 1) / 2 + i + 1);
}

static void multiply_av(int n, const double *v, double *av) {
    for (int i = 0; i < n; i++) {
        double sum = 0.0;
        for (int j = 0; j < n; j++) sum += v[j] / a(i, j);
        av[i] = sum;
    }
}

static void multiply_atv(int n, const double *v, double *atv) {
    for (int i = 0; i < n; i++) {
        double sum = 0.0;
        for (int j = 0; j < n; j++) sum += v[j] / a(j, i);
        atv[i] = sum;
    }
}

static void multiply_atav(int n, const double *v, double *tmp, double *out) {
    multiply_av(n, v, tmp);
    multiply_atv(n, tmp, out);
}

int main(void) {
    const int n = 5500;
    double *u = malloc(n * sizeof(double));
    double *v = malloc(n * sizeof(double));
    double *tmp = malloc(n * sizeof(double));
    if (!u || !v || !tmp) abort();
    for (int i = 0; i < n; i++) u[i] = 1.0;
    for (int i = 0; i < 10; i++) {
        multiply_atav(n, u, tmp, v);
        multiply_atav(n, v, tmp, u);
    }
    double vbv = 0.0, vv = 0.0;
    for (int i = 0; i < n; i++) {
        vbv += u[i] * v[i];
        vv += v[i] * v[i];
    }
    printf("%.9f\n", sqrt(vbv / vv));
    free(u);
    free(v);
    free(tmp);
    return 0;
}
