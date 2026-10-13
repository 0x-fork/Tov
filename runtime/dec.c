/* dec.c — `decimal`: exact base-10 numbers for money and the like.
 *
 * A decimal is coefficient × 10^-scale, the coefficient an integer of at most 34 digits (as IEEE
 * decimal128's) and the scale 0..34. Both sit in 128 bits: the coefficient (signed) shifted left
 * by 8, the scale in the low byte. Values with the same scale add, subtract and compare as single
 * 128-bit integers (tov.h does those inline); everything else (aligning scales, a product past
 * 34 digits, division, rounding) goes through the limb arithmetic of big.c (this file follows it
 * in the runtime's translation unit).
 *
 * Results are exact while they fit: `+`, `-` and `*` keep every digit (the scale of a sum is the
 * larger of the two, of a product their sum), as SQL's NUMERIC and Java's BigDecimal do, so
 * 1.10 + 2.205 is 3.305 and 1.10 * 3 is 3.30. A result past 34 digits is rounded to 34, half to
 * even. Division gives 34 significant digits (half to even), with trailing zeros dropped down to
 * the dividend's scale less the divisor's when it's exact: 10.00 / 4 is 2.50. A value whose
 * integer part needs more than 34 digits traps. */

#include "tov.h"

#include <stdlib.h>
#include <string.h>

typedef __int128 tv_i128;
typedef unsigned __int128 tv_u128;

#define TV_DEC_DIGITS 34

const uint64_t tv_p10[20] = {
    1ull, 10ull, 100ull, 1000ull, 10000ull, 100000ull, 1000000ull, 10000000ull, 100000000ull, 1000000000ull,
    10000000000ull, 100000000000ull, 1000000000000ull, 10000000000000ull, 100000000000000ull, 1000000000000000ull,
    10000000000000000ull, 100000000000000000ull, 1000000000000000000ull, 10000000000000000000ull,
};

static inline tv_u128 tv_pow10_u128(int k) { /* k <= 38 */
    return k <= 19 ? (tv_u128)tv_p10[k] : (tv_u128)tv_p10[19] * tv_p10[k - 19];
}

/* Digits of a 128-bit magnitude (1 for 0): from its bit length, then one comparison. */
static int tv_u128_digits(tv_u128 m) {
    uint64_t hi = (uint64_t)(m >> 64);
    int bits = hi ? 128 - __builtin_clzll(hi) : ((uint64_t)m ? 64 - __builtin_clzll((uint64_t)m) : 1);
    int d = (bits * 1233) >> 12;   /* floor(bits * log10(2)), or one less */
    return m >= tv_pow10_u128(d) ? d + 1 : (d ? d : 1);
}

/* 10^k (k = 1..19) normalized, with its shift and reciprocal: m / 10^k without a division. */
static uint64_t tv_p10n[20], tv_p10i[20];
static int tv_p10s[20];
static void tv_p10_init(void) {
    for (int k = 1; k < 20; k++) {
        tv_p10s[k] = __builtin_clzll(tv_p10[k]);
        tv_p10n[k] = tv_p10[k] << tv_p10s[k];
        tv_p10i[k] = tv_recip(tv_p10n[k]);
    }
}

/* m / 10^k for a 128-bit m, k = 1..19: the quotient and remainder. */
static inline tv_u128 tv_u128_div_p10(tv_u128 m, int k, uint64_t *rem) {
    if (TV_BIG_UNLIKELY(!tv_p10i[1])) tv_p10_init();
    int s = tv_p10s[k];
    uint64_t dn = tv_p10n[k], di = tv_p10i[k], hi = (uint64_t)(m >> 64), lo = (uint64_t)m, r;
    /* (hi, lo) << s as three limbs: n2 n1 n0 */
    uint64_t n2 = s ? hi >> (64 - s) : 0, n1 = s ? (hi << s) | (lo >> (64 - s)) : hi, n0 = lo << s;
    uint64_t q1 = tv_div_preinv(n2, n1, dn, di, &r);
    uint64_t q0 = tv_div_preinv(r, n0, dn, di, &r);
    *rem = r >> s;
    return ((tv_u128)q1 << 64) | q0;
}

/* ---------------------------------------------------------------- wide magnitudes */

/* A magnitude of up to 8 limbs (scaled operands and quotients stay well inside). */
typedef struct { tv_limb d[8]; uint32_t n; } tv_wide;

static void tv_wide_set(tv_wide *w, tv_u128 m) {
    w->d[0] = (tv_limb)m;
    w->d[1] = (tv_limb)(m >> 64);
    w->n = w->d[1] ? 2 : (w->d[0] ? 1 : 0);
}

/* w *= 10^k */
static void tv_wide_scale(tv_wide *w, int k) {
    while (k > 0 && w->n) {
        int step = k > 19 ? 19 : k;
        tv_limb c = tv_mag_mul1(w->d, w->d, w->n, tv_p10[step]);
        if (c) w->d[w->n++] = c;
        k -= step;
    }
}

/* w /= 10^k; returns the remainder's first dropped digit (0..9) and sets *sticky if anything
 * below it was nonzero. */
static int tv_wide_drop(tv_wide *w, int k, bool *sticky) {
    int digit = 0;
    while (k > 0) {
        int step = k > 19 ? 19 : k;
        tv_limb r = w->n ? tv_mag_divrem1(w->d, w->d, w->n, tv_p10[step]) : 0;
        w->n = tv_mag_norm(w->d, w->n);
        k -= step;
        if (k == 0) {
            /* the last step's top digit is the rounding digit */
            digit = (int)(r / tv_p10[step - 1]);
            if (r % tv_p10[step - 1]) *sticky = true;
        } else if (r) {
            *sticky = true;
        }
    }
    return digit;
}

static bool tv_wide_odd(const tv_wide *w) { return w->n && (w->d[0] & 1); }

static void tv_wide_inc(tv_wide *w) {
    tv_limb one = 1;
    if (!w->n) {
        w->d[0] = 1;
        w->n = 1;
        return;
    }
    tv_limb c = tv_mag_add(w->d, w->d, w->n, &one, 1);
    if (c) w->d[w->n++] = c;
}

/* Fits in 34 digits? (as a u128 when so) */
static bool tv_wide_small(const tv_wide *w, tv_u128 *out) {
    if (w->n > 2) return false;
    tv_u128 m = w->n == 0 ? 0 : w->n == 1 ? w->d[0] : (((tv_u128)w->d[1] << 64) | w->d[0]);
    if (m >= tv_pow10_u128(TV_DEC_DIGITS)) return false;
    *out = m;
    return true;
}

static int tv_wide_digits(const tv_wide *w) {
    if (w->n <= 2) {
        tv_u128 m = w->n == 0 ? 0 : w->n == 1 ? w->d[0] : (((tv_u128)w->d[1] << 64) | w->d[0]);
        return tv_u128_digits(m);
    }
    uint32_t bits = w->n * 64 - (uint32_t)__builtin_clzll(w->d[w->n - 1]);
    /* floor(bits * log10(2)) or one more */
    int d = (int)((uint64_t)bits * 30103 / 100000);
    tv_wide t = *w;
    bool st = false;
    if (d > 0) tv_wide_drop(&t, d, &st);
    /* (t, what's above 10^d, is under 20) */
    tv_limb top = t.n ? t.d[0] : 0;
    return top >= 10 ? d + 2 : top ? d + 1 : d;
}

/* ---------------------------------------------------------------- rounding */

enum { TV_RM_HALF_EVEN, TV_RM_HALF_EXPAND, TV_RM_HALF_TRUNC, TV_RM_HALF_CEIL, TV_RM_HALF_FLOOR, TV_RM_CEIL, TV_RM_FLOOR, TV_RM_TRUNC, TV_RM_EXPAND };

/* Should the dropped part (first digit, anything below) round the kept magnitude up? */
static bool tv_round_up(int mode, int digit, bool sticky, bool odd, bool neg) {
    bool above_half = digit > 5 || (digit == 5 && sticky), half = digit == 5 && !sticky, any = digit || sticky;
    switch (mode) {
    case TV_RM_HALF_EVEN: return above_half || (half && odd);
    case TV_RM_HALF_EXPAND: return digit >= 5;
    case TV_RM_HALF_TRUNC: return above_half;
    case TV_RM_HALF_CEIL: return above_half || (half && !neg);
    case TV_RM_HALF_FLOOR: return above_half || (half && neg);
    case TV_RM_CEIL: return any && !neg;
    case TV_RM_FLOOR: return any && neg;
    case TV_RM_EXPAND: return any;
    default: return false;
    }
}

/* The decimal m × 10^-s (sign neg), rounded to 34 digits and a scale of at most `max_scale`
 * (and at least 0); `sticky`: something nonzero was dropped already, below m's last digit. */
static bool tv_dec_inexact;   /* the last tv_dec_finish dropped something nonzero */

static tv_dec tv_dec_finish(tv_wide *m, int s, bool neg, bool sticky, int mode, int max_scale, const char *loc) {
    tv_dec_inexact = sticky;
    if (s < 0) {
        tv_wide_scale(m, -s);
        s = 0;
    }
    int drop = s > max_scale ? s - max_scale : 0;
    int digits = tv_wide_digits(m);
    if (digits - drop > TV_DEC_DIGITS) drop = digits - TV_DEC_DIGITS;
    if (drop > s) tv_trap("decimal overflow: more than 34 digits before the point", loc);
    if (drop > 0) {
        int digit = tv_wide_drop(m, drop, &sticky);
        if (digit || sticky) tv_dec_inexact = true;
        s -= drop;
        if (tv_round_up(mode, digit, sticky, tv_wide_odd(m), neg)) tv_wide_inc(m);
    } else if (sticky && tv_round_up(mode, 0, true, tv_wide_odd(m), neg)) {
        tv_wide_inc(m);
    }
    tv_u128 v;
    if (!tv_wide_small(m, &v)) {
        /* rounding carried into a 35th digit (99...9 -> 100...0): one zero less */
        if (s == 0) tv_trap("decimal overflow: more than 34 digits before the point", loc);
        bool st = false;
        tv_wide_drop(m, 1, &st);
        s--;
        tv_wide_small(m, &v);
    }
    return tv_dec_make(neg && v ? -(tv_i128)v : (tv_i128)v, s);
}

static void tv_dec_wide(tv_dec a, tv_wide *w, bool *neg) {
    tv_i128 c = tv_dec_coef(a);
    *neg = c < 0;
    tv_wide_set(w, c < 0 ? (tv_u128)0 - (tv_u128)c : (tv_u128)c);
}

/* ---------------------------------------------------------------- arithmetic */

tv_dec tv_dec_add_slow(tv_dec a, tv_dec b, int sign) {
    int sa = tv_dec_scale(a), sb = tv_dec_scale(b), s = sa > sb ? sa : sb;
    {
        /* scales a few digits apart (0.30 + 1.2345): the smaller-scale side times 10^k in 128 bits */
        tv_i128 x = tv_dec_coef(a), y = tv_dec_coef(b);
        int k = s - (sa < sb ? sa : sb);
        tv_i128 *lo = sa < sb ? &x : &y;
        if (k <= 19 && *lo == (int64_t)*lo) {
            *lo *= (tv_i128)tv_p10[k];
            tv_i128 c = sign > 0 ? x + y : x - y;
            if (c < TV_DEC_LIM && c > -TV_DEC_LIM) return tv_dec_make(c, s);
        }
    }
    tv_wide x, y;
    bool xn, yn;
    tv_dec_wide(a, &x, &xn);
    tv_dec_wide(b, &y, &yn);
    if (sign < 0) yn = !yn;
    tv_wide_scale(&x, s - sa);
    tv_wide_scale(&y, s - sb);
    tv_wide r;
    memset(&r, 0, sizeof r);
    bool rn;
    if (xn == yn) {
        const tv_wide *p = x.n >= y.n ? &x : &y, *q = x.n >= y.n ? &y : &x;
        r.n = p->n;
        tv_limb c = q->n ? tv_mag_add(r.d, p->d, p->n, q->d, q->n) : (memcpy(r.d, p->d, p->n * sizeof(tv_limb)), 0);
        if (c) r.d[r.n++] = c;
        rn = xn;
    } else {
        int c = tv_mag_cmp(x.d, x.n, y.d, y.n);
        const tv_wide *p = c >= 0 ? &x : &y, *q = c >= 0 ? &y : &x;
        r.n = p->n;
        if (q->n) tv_mag_sub(r.d, p->d, p->n, q->d, q->n);
        else memcpy(r.d, p->d, p->n * sizeof(tv_limb));
        r.n = tv_mag_norm(r.d, r.n);
        rn = c >= 0 ? xn : yn;
    }
    return tv_dec_finish(&r, s, rn, false, TV_RM_HALF_EVEN, TV_DEC_DIGITS, "decimal arithmetic");
}

tv_dec tv_dec_mul_slow(tv_dec a, tv_dec b) {
    tv_wide x, y, r;
    bool xn, yn;
    tv_dec_wide(a, &x, &xn);
    tv_dec_wide(b, &y, &yn);
    int s = tv_dec_scale(a) + tv_dec_scale(b);
    if (!x.n || !y.n) return tv_dec_make(0, s > TV_DEC_DIGITS ? TV_DEC_DIGITS : s);
    memset(&r, 0, sizeof r);
    tv_mag_mul_school(r.d, x.d, x.n, y.d, y.n);
    r.n = tv_mag_norm(r.d, x.n + y.n);
    return tv_dec_finish(&r, s, xn != yn, false, TV_RM_HALF_EVEN, TV_DEC_DIGITS, "decimal arithmetic");
}

/* Strips trailing zeros of an exact result down to scale `keep`. */
static tv_dec tv_dec_trim(tv_dec d, int keep) {
    tv_i128 c = tv_dec_coef(d);
    int s = tv_dec_scale(d);
    while (s > keep && c % 10 == 0) {
        c /= 10;
        s--;
    }
    return tv_dec_make(c, s);
}

/* n / d for n < d * 2^64 (d normalized by `s`, reciprocal di): a quotient limb and remainder. */
static inline uint64_t tv_div128(tv_u128 n, int s, uint64_t dn, uint64_t di, uint64_t *rem) {
    tv_u128 nn = n << s;
    uint64_t r;
    uint64_t q = tv_div_preinv((uint64_t)(nn >> 64), (uint64_t)nn, dn, di, &r);
    *rem = r >> s;
    return q;
}

/* Division by a coefficient under 2^64 (prices, totals, counts): long division 19 digits at a
 * time, each chunk one 128-by-64 division by reciprocal, until 35 significant digits or an exact
 * result. false if the case doesn't fit (the general path takes it). */
static bool tv_dec_div_fast(tv_dec a, tv_dec b, tv_dec *out) {
    tv_i128 ca = tv_dec_coef(a), cb = tv_dec_coef(b);
    bool neg = (ca < 0) != (cb < 0);
    tv_u128 x = ca < 0 ? (tv_u128)0 - (tv_u128)ca : (tv_u128)ca, yy = cb < 0 ? (tv_u128)0 - (tv_u128)cb : (tv_u128)cb;
    if (yy >> 64) return false;
    uint64_t y = (uint64_t)yy;
    if (y % 10 == 0 || y == 1) {
        /* a power of ten (x / 100): only the scale changes */
        int j = 0;
        while (j < 19 && tv_p10[j] < y) j++;
        if (tv_p10[j] == y) {
            int base = tv_dec_scale(a) - tv_dec_scale(b), scale = base + j, preferred = base < 0 ? 0 : base;
            if (scale > 34) return false;
            if (scale < 0) {
                if (x && tv_u128_digits(x) - scale > 34) return false;
                x *= tv_pow10_u128(-scale);
                scale = 0;
            }
            while (scale > preferred && x && x % 10 == 0) {
                x /= 10;
                scale--;
            }
            *out = tv_dec_make(neg ? -(tv_i128)x : (tv_i128)x, x ? scale : preferred);
            return true;
        }
    }
    int s = __builtin_clzll(y);
    uint64_t dn = y << s, di = tv_recip(dn), r;
    /* the integer part, in two limbs */
    uint64_t x1 = (uint64_t)(x >> 64), q1 = x1 / y;
    uint64_t q0 = tv_div128(((tv_u128)(x1 - q1 * y) << 64) | (uint64_t)x, s, dn, di, &r);
    tv_u128 acc = ((tv_u128)q1 << 64) | q0;
    int sig = acc ? tv_u128_digits(acc) : 0, frac = 0;
    int base = tv_dec_scale(a) - tv_dec_scale(b);
    /* fractional digits, a chunk at a time, while they're significant and the scale allows */
    while (r != 0 && sig < 35 && base + frac < 35) {
        int k = 35 - sig;
        if (k > 35 - (base + frac)) k = 35 - (base + frac);
        if (k > 19) k = 19;
        uint64_t d = tv_div128((tv_u128)r * tv_p10[k], s, dn, di, &r);
        if (r == 0) {
            /* exact: drop the chunk's trailing zeros */
            while (k > 0 && d % 10 == 0) {
                d /= 10;
                k--;
            }
        }
        if (acc) {
            acc = acc * tv_p10[k] + d;
            sig += k;
        } else {
            acc = d;
            sig = d ? tv_u128_digits(d) : 0;
        }
        frac += k;
    }
    int scale = base + frac;
    bool sticky = r != 0;
    int drop = sig > 34 ? sig - 34 : 0;
    if (scale - drop > 34) drop = scale - 34;
    if (drop > 0) {
        if (drop > 19) return false;
        uint64_t p = tv_p10[drop], rem;
        tv_u128 q = tv_u128_div_p10(acc, drop, &rem);
        bool up = 2 * (tv_u128)rem > p || (2 * (tv_u128)rem == p && (sticky || (q & 1)));
        acc = q + up;
        scale -= drop;
        if (acc >= tv_pow10_u128(34)) {
            acc = tv_u128_div_p10(acc, 1, &rem);
            scale--;
        }
    } else if (sticky) {
        /* (35 digits weren't reached: the scale cap stopped it; the rest is below half an ulp
         * only if...) let the general path round it */
        return false;
    }
    if (scale < 0) {
        if (scale < -34 || (acc && tv_u128_digits(acc) - scale > 34)) return false;
        acc *= tv_pow10_u128(-scale);
        scale = 0;
    }
    /* an exact result with no fraction left keeps the dividend's scale less the divisor's */
    if (!sticky && scale < base) {
        if (base > 34 || (acc && tv_u128_digits(acc) + (base - scale) > 34)) return false;
        acc *= tv_pow10_u128(base - scale);
        scale = base;
    }
    *out = tv_dec_make(neg ? -(tv_i128)acc : (tv_i128)acc, scale);
    return true;
}

static bool tv_dec_div_general;   /* (tests compare the two paths) */
tv_dec tv_dec_div(tv_dec a, tv_dec b, const char *loc);

/* (a / b).round(k, mode) straight to k places. Rounding the exact quotient once gives what
 * rounding it to 34 digits first and then to k places does unless the quotient lies within half
 * a unit of the 34th digit of a k-place boundary; a quotient of a divisor b is at least
 * 1 / (2 · 10^k · b) away from any such point it isn't on, so for b < 10^(34 - k - its integer
 * digits) both agree, and that's the case taken here (anything else divides, then rounds). */
tv_dec tv_dec_div_round(tv_dec a, tv_dec b, tv_int k, int mode, const char *loc) {
    tv_i128 ca = tv_dec_coef(a), cb = tv_dec_coef(b);
    if (cb == 0) tv_trap("decimal division by zero", loc);
    int sa = tv_dec_scale(a), sb = tv_dec_scale(b);
    tv_u128 x = ca < 0 ? (tv_u128)0 - (tv_u128)ca : (tv_u128)ca, y = cb < 0 ? (tv_u128)0 - (tv_u128)cb : (tv_u128)cb;
    int e = sb + (int)k - sa;   /* Q * 10^k = x * 10^e / y */
    if (k >= 0 && k <= 34 && e >= -19 && e <= 19 && !(y >> 64) && !tv_dec_div_general) {
        if (e < 0) {
            y *= tv_p10[-e];
            if (y >> 64) goto slow;
        } else if (x) {
            if (tv_u128_digits(x) + e > 38) goto slow;
            x *= tv_p10[e];
        }
        uint64_t yd = (uint64_t)y;
        int s = __builtin_clzll(yd);
        uint64_t dn = yd << s, di = tv_recip(dn), r;
        uint64_t x1 = (uint64_t)(x >> 64), q1 = x1 / yd;
        uint64_t q0 = tv_div128(((tv_u128)(x1 - q1 * yd) << 64) | (uint64_t)x, s, dn, di, &r);
        tv_u128 q = ((tv_u128)q1 << 64) | q0;
        /* the quotient's integer digits: q has k digits after the point */
        int qd = q ? tv_u128_digits(q) : 0, intd = qd > k ? qd - (int)k : 0;
        int room = 34 - (int)k - intd;   /* b (here y, scaled alike) must be under 10^room */
        if (room <= 0 || (room < 39 && y >= tv_pow10_u128(room))) goto slow;
        bool neg = (ca < 0) != (cb < 0);
        if (r != 0) {
            int digit = 2 * (tv_u128)r > (tv_u128)yd ? 6 : 2 * (tv_u128)r == (tv_u128)yd ? 5 : 0;
            if (tv_round_up(mode, digit, digit == 0, (uint64_t)q & 1, neg)) q++;
            if (q >= tv_pow10_u128(34)) goto slow;
            return tv_dec_make(neg ? -(tv_i128)q : (tv_i128)q, (int)k);
        }
        /* exact: the scale the division would have kept (no more than k) */
        int preferred = sa - sb < 0 ? 0 : sa - sb, scale = (int)k;
        while (scale > preferred && q && q % 10 == 0) {
            q /= 10;
            scale--;
        }
        return tv_dec_make(neg ? -(tv_i128)q : (tv_i128)q, q ? scale : (preferred < k ? preferred : (int)k));
    }
slow:
    return tv_dec_round_slow(tv_dec_div(a, b, loc), k, mode, loc);
}

tv_dec tv_dec_div(tv_dec a, tv_dec b, const char *loc) {
    tv_i128 cb = tv_dec_coef(b);
    if (cb == 0) tv_trap("decimal division by zero", loc);
    tv_dec fast;
    if (!tv_dec_div_general && tv_dec_div_fast(a, b, &fast)) return fast;
    int sa = tv_dec_scale(a), sb = tv_dec_scale(b);
    tv_wide x, y, q;
    bool xn, yn;
    tv_dec_wide(a, &x, &xn);
    tv_dec_wide(b, &y, &yn);
    int preferred = sa - sb < 0 ? 0 : sa - sb;
    if (!x.n) return tv_dec_make(0, preferred);
    /* x * 10^e / y with at least 35 digits, so 34 survive rounding */
    int e = TV_DEC_DIGITS + 1 - tv_wide_digits(&x) + tv_wide_digits(&y);
    if (e < 0) e = 0;
    tv_wide_scale(&x, e);
    memset(&q, 0, sizeof q);
    bool sticky;
    if (y.n == 1) {
        tv_limb r = tv_mag_divrem1(q.d, x.d, x.n, y.d[0]);
        q.n = tv_mag_norm(q.d, x.n);
        sticky = r != 0;
    } else {
        tv_limb rem[8];
        if (tv_mag_cmp(x.d, x.n, y.d, y.n) < 0) {
            q.n = 0;
            sticky = true;
        } else {
            tv_mag_divrem(q.d, rem, x.d, x.n, y.d, y.n);
            q.n = tv_mag_norm(q.d, x.n - y.n + 1);
            sticky = tv_mag_norm(rem, y.n) != 0;
        }
    }
    tv_dec r = tv_dec_finish(&q, sa - sb + e, xn != yn, sticky, TV_RM_HALF_EVEN, TV_DEC_DIGITS, loc);
    return tv_dec_inexact ? r : tv_dec_trim(r, preferred);
}

tv_dec tv_dec_rem(tv_dec a, tv_dec b, const char *loc) {
    if (tv_dec_coef(b) == 0) tv_trap("decimal remainder by zero", loc);
    int sa = tv_dec_scale(a), sb = tv_dec_scale(b), s = sa > sb ? sa : sb;
    tv_wide x, y, r;
    bool xn, yn;
    tv_dec_wide(a, &x, &xn);
    tv_dec_wide(b, &y, &yn);
    tv_wide_scale(&x, s - sa);
    tv_wide_scale(&y, s - sb);
    memset(&r, 0, sizeof r);
    if (tv_mag_cmp(x.d, x.n, y.d, y.n) < 0) {
        r = x;
    } else if (y.n == 1) {
        tv_limb q[8];
        r.d[0] = tv_mag_divrem1(q, x.d, x.n, y.d[0]);
        r.n = r.d[0] != 0;
    } else {
        tv_limb q[8];
        tv_mag_divrem(q, r.d, x.d, x.n, y.d, y.n);
        r.n = tv_mag_norm(r.d, y.n);
    }
    return tv_dec_finish(&r, s, xn, false, TV_RM_HALF_EVEN, TV_DEC_DIGITS, loc);
}

int tv_dec_cmp_slow(tv_dec a, tv_dec b) {
    int sa = tv_dec_scale(a), sb = tv_dec_scale(b), s = sa > sb ? sa : sb;
    tv_wide x, y;
    bool xn, yn;
    tv_dec_wide(a, &x, &xn);
    tv_dec_wide(b, &y, &yn);
    if (!x.n) xn = false;
    if (!y.n) yn = false;
    if (xn != yn) return xn ? -1 : 1;
    tv_wide_scale(&x, s - sa);
    tv_wide_scale(&y, s - sb);
    int c = tv_mag_cmp(x.d, x.n, y.d, y.n);
    return xn ? -c : c;
}

uint64_t tv_dec_hash(tv_dec a) {
    /* equal values hash alike whatever their scale: 1.10 and 1.1 */
    tv_dec t = tv_dec_trim(a, 0);
    uint64_t h = (t.lo * 0x9E3779B97F4A7C15ull) ^ (t.hi + 0x632BE59BD9B4E019ull);
    return h ^ (h >> 31);
}

/* ---------------------------------------------------------------- rounding */

static int tv_dec_mode(tv_str mode) {
    static const char *const names[] = { "halfEven", "halfExpand", "halfTrunc", "halfCeil", "halfFloor", "ceil", "floor", "trunc", "expand" };
    for (int i = 0; i < 9; i++) {
        if (strcmp(mode.p->data, names[i]) == 0) return i;
    }
    return TV_RM_HALF_EVEN;
}

/* `places` digits after the point, at most (negative: rounds to tens, hundreds...). */
static tv_dec tv_dec_round_mode(tv_dec a, tv_int places, int mode, const char *loc) {
    if (places < -TV_DEC_DIGITS || places > TV_DEC_DIGITS) tv_trap("decimal round(): places must be between -34 and 34", loc);
    int s = tv_dec_scale(a);
    if (places >= s) return a;
    tv_i128 c = tv_dec_coef(a);
    if (places >= 0 && c != (int64_t)c) {
        /* a long coefficient: divisions by 10^19 at most, by reciprocal (the first ones only say
         * whether anything nonzero was dropped) */
        bool neg = c < 0, below = false;
        tv_u128 q = neg ? (tv_u128)0 - (tv_u128)c : (tv_u128)c;
        int drop = s - (int)places;
        uint64_t r;
        while (drop > 19) {
            q = tv_u128_div_p10(q, 19, &r);
            below |= r != 0;
            drop -= 19;
        }
        uint64_t p = tv_p10[drop];
        q = tv_u128_div_p10(q, drop, &r);
        int digit = r == 0 ? 0 : 2 * (tv_u128)r > p ? 6 : 2 * (tv_u128)r == p ? 5 : 1;
        if (digit == 0 && below) digit = 1;
        bool sticky = digit == 1 || (digit == 5 && below) || (digit == 6 && below);
        if (digit == 5 && below) digit = 6;
        if (digit == 1) digit = 0;
        if (tv_round_up(mode, digit, sticky, (uint64_t)q & 1, neg)) q++;
        return tv_dec_make(neg ? -(tv_i128)q : (tv_i128)q, (int)places);
    }
    if (places >= 0 && s - places <= 18 && c == (int64_t)c) {
        /* the usual case, in 64 bits: q = |c| / 10^drop, and how the rest compares with half */
        bool neg = c < 0;
        uint64_t ax = neg ? (uint64_t)0 - (uint64_t)(int64_t)c : (uint64_t)(int64_t)c, p = tv_p10[s - places];
        uint64_t q = ax / p, r = ax - q * p;
        int digit = r == 0 ? 0 : 2 * r > p ? 6 : 2 * r == p ? 5 : 1;
        bool sticky = digit == 1;
        if (digit == 1) digit = 0;
        if (tv_round_up(mode, digit, sticky, q & 1, neg)) q++;
        return tv_dec_make(neg ? -(tv_i128)q : (tv_i128)q, (int)places);
    }
    tv_wide m;
    bool neg;
    tv_dec_wide(a, &m, &neg);
    int drop = s - (int)places;
    bool sticky = false;
    int digit = tv_wide_drop(&m, drop, &sticky);
    if (tv_round_up(mode, digit, sticky, tv_wide_odd(&m), neg)) tv_wide_inc(&m);
    return tv_dec_finish(&m, (int)places, neg, false, TV_RM_HALF_EVEN, TV_DEC_DIGITS, loc);
}

tv_dec tv_dec_round(tv_dec a, tv_int places, tv_str mode, const char *loc) { return tv_dec_round_mode(a, places, tv_dec_mode(mode), loc); }
tv_dec tv_dec_round_slow(tv_dec a, tv_int places, int mode, const char *loc) { return tv_dec_round_mode(a, places, mode, loc); }
tv_dec tv_dec_floor(tv_dec a) { return tv_dec_round_mode(a, 0, TV_RM_FLOOR, ""); }
tv_dec tv_dec_ceil(tv_dec a) { return tv_dec_round_mode(a, 0, TV_RM_CEIL, ""); }
tv_dec tv_dec_trunc(tv_dec a) { return tv_dec_round_mode(a, 0, TV_RM_TRUNC, ""); }

/* ---------------------------------------------------------------- text */

static size_t tv_u128_to_chars(tv_u128 m, char *buf) { /* buf >= 40; returns the length */
    char tmp[40];
    size_t n = 0;
    do {
        uint64_t chunk;
        if (m >= tv_p10[19]) {
            chunk = (uint64_t)(m % tv_p10[19]);
            m /= tv_p10[19];
            for (int i = 0; i < 19; i++) {
                tmp[n++] = (char)('0' + chunk % 10);
                chunk /= 10;
            }
        } else {
            chunk = (uint64_t)m;
            m = 0;
            do {
                tmp[n++] = (char)('0' + chunk % 10);
                chunk /= 10;
            } while (chunk);
        }
    } while (m);
    for (size_t i = 0; i < n; i++) buf[i] = tmp[n - 1 - i];
    return n;
}

void tv_sb_push_dec(tv_sb *sb, tv_dec a) {
    tv_i128 c = tv_dec_coef(a);
    int s = tv_dec_scale(a);
    char digits[40], out[80];
    size_t n = tv_u128_to_chars(c < 0 ? (tv_u128)0 - (tv_u128)c : (tv_u128)c, digits), k = 0;
    if (c < 0) out[k++] = '-';
    if ((int)n <= s) {
        out[k++] = '0';
        out[k++] = '.';
        for (int i = 0; i < s - (int)n; i++) out[k++] = '0';
        memcpy(out + k, digits, n);
        k += n;
    } else {
        memcpy(out + k, digits, n - (size_t)s);
        k += n - (size_t)s;
        if (s) {
            out[k++] = '.';
            memcpy(out + k, digits + n - s, (size_t)s);
            k += (size_t)s;
        }
    }
    tv_sb_push(sb, out, k);
}

tv_str tv_dec_to_string(tv_dec a) {
    tv_sb sb = {0};
    tv_sb_push_dec(&sb, a);
    return tv_str_from_sb(&sb);
}

/* toFixed(digits): rounded half away from zero (as Number's toFixed), exactly `digits` places. */
tv_str tv_dec_to_fixed(tv_dec a, tv_int digits, const char *loc) {
    if (digits < 0 || digits > TV_DEC_DIGITS) tv_trap("toFixed() digits must be between 0 and 34", loc);
    tv_dec r = tv_dec_round_mode(a, digits, TV_RM_HALF_EXPAND, loc);
    int s = tv_dec_scale(r);
    if (s < digits) {
        /* pad with zeros: the coefficient times 10^(digits - s), if it fits; else as text */
        tv_sb sb = {0};
        tv_sb_push_dec(&sb, r);
        if (s == 0) tv_sb_push_char(&sb, '.');
        for (tv_int i = s; i < digits; i++) tv_sb_push_char(&sb, '0');
        return tv_str_from_sb(&sb);
    }
    return tv_dec_to_string(r);
}

/* Parses s[0, n): [sign] digits [. digits] [e [sign] digits]. 0: done, 1: not a number,
 * 2: too large. More than 34 significant digits round half to even. */
int tv_dec_parse_n(const char *s, size_t n, tv_dec *out) {
    if (n && n <= 19) {
        /* the usual case: [-]digits[.digits], 18 digits at most, in 64 bits */
        size_t j = 0;
        bool neg = s[0] == '-';
        if (s[0] == '-' || s[0] == '+') j++;
        uint64_t m = 0;
        int scale = -1, nd = 0;
        for (; j < n; j++) {
            unsigned d = (unsigned char)s[j] - '0';
            if (d < 10) {
                m = m * 10 + d;
                nd++;
                if (scale >= 0) scale++;
            } else if (s[j] == '.' && scale < 0) {
                scale = 0;
            } else {
                break;
            }
        }
        if (j == n && nd > 0 && nd <= 18) {
            *out = tv_dec_make(neg ? -(tv_i128)m : (tv_i128)m, scale < 0 ? 0 : scale);
            return 0;
        }
    }
    size_t i = 0;
    bool neg = false;
    if (i < n && (s[i] == '+' || s[i] == '-')) neg = s[i++] == '-';
    tv_u128 m = 0;
    int kept = 0, scale = 0, digit = -1;
    bool any = false, sticky = false, frac = false;
    for (; i < n; i++) {
        char ch = s[i];
        if (ch == '.' && !frac) {
            frac = true;
            continue;
        }
        if (ch == '_' && any && i + 1 < n && s[i + 1] >= '0' && s[i + 1] <= '9') continue;
        if (ch < '0' || ch > '9') break;
        any = true;
        int d = ch - '0';
        if (kept < TV_DEC_DIGITS) {
            if (m == 0 && d == 0) {
                if (frac) scale++;
                continue;
            }
            m = m * 10 + (tv_u128)d;
            kept++;
            if (frac) scale++;
        } else {
            /* past 34 digits: the first one rounds, the rest only say whether it's exact */
            if (digit < 0) digit = d;
            else if (d) sticky = true;
            if (!frac) scale--;
        }
    }
    if (!any) return 1;
    if (i < n && (s[i] == 'e' || s[i] == 'E')) {
        i++;
        bool eneg = false;
        if (i < n && (s[i] == '+' || s[i] == '-')) eneg = s[i++] == '-';
        if (i >= n || s[i] < '0' || s[i] > '9') return 1;
        long e = 0;
        for (; i < n && s[i] >= '0' && s[i] <= '9'; i++) {
            if (e < 100000) e = e * 10 + (s[i] - '0');
        }
        scale += eneg ? (int)e : -(int)e;
    }
    if (i != n) return 1;
    if (digit >= 0) {
        bool odd = (m & 1) != 0;
        if (tv_round_up(TV_RM_HALF_EVEN, digit, sticky, odd, neg)) m++;
        if (m >= tv_pow10_u128(TV_DEC_DIGITS)) {
            m /= 10;
            scale--;
        }
    }
    if (scale > TV_DEC_DIGITS) {
        /* digits below the 34th place: round them off */
        tv_wide w;
        tv_wide_set(&w, m);
        bool st = false;
        int dg = tv_wide_drop(&w, scale - TV_DEC_DIGITS, &st);
        if (tv_round_up(TV_RM_HALF_EVEN, dg, st, tv_wide_odd(&w), neg)) tv_wide_inc(&w);
        tv_wide_small(&w, &m);
        scale = TV_DEC_DIGITS;
    }
    if (scale < 0) {
        if (m != 0 && tv_u128_digits(m) - scale > TV_DEC_DIGITS) return 2;
        m *= tv_pow10_u128(-scale);
        scale = 0;
    }
    *out = tv_dec_make(neg ? -(tv_i128)m : (tv_i128)m, scale);
    return 0;
}

static bool tv_dec_space(unsigned char c) { return c == ' ' || (c >= '\t' && c <= '\r'); }

/* decimal(string): surrounding whitespace allowed. 0, 1 (SyntaxError) or 2 (RangeError). */
int tv_dec_parse(tv_str s, tv_dec *out) {
    const char *p = s.p->data, *e = p + s.p->len;
    while (p < e && tv_dec_space((unsigned char)*p)) p++;
    while (e > p && tv_dec_space((unsigned char)e[-1])) e--;
    return tv_dec_parse_n(p, (size_t)(e - p), out);
}

/* A literal (the compiler checked it). */
tv_dec tv_dec_lit(const char *text) {
    tv_dec d = tv_dec_make(0, 0);
    tv_dec_parse_n(text, strlen(text), &d);
    return d;
}

/* decimal(number): the number's shortest decimal form (decimal(0.1) is 0.1); false for NaN and
 * the infinities. */
bool tv_dec_from_f64(double x, tv_dec *out) {
    if (x != x || x - x != 0) return false;
    tv_sb sb = {0};
    tv_sb_push_f64(&sb, x);
    int r = tv_dec_parse_n(sb.data, sb.len, out);
    tv_sb_free(&sb);
    return r == 0;
}

double tv_dec_to_f64(tv_dec a) {
    char buf[96];
    tv_sb sb = {0};
    tv_sb_push_dec(&sb, a);
    size_t n = sb.len < sizeof buf - 1 ? sb.len : sizeof buf - 1;
    memcpy(buf, sb.data, n);
    buf[n] = 0;
    tv_sb_free(&sb);
    return strtod(buf, NULL);
}

tv_dec tv_dec_from_big(tv_big b, const char *loc) {
    if (tv_big_is_small(b)) return tv_dec_make((tv_i128)(b.v >> 1), 0);
    tv_bigbuf *x = tv_bb(b);
    if (x->len > 2) tv_trap("decimal overflow: more than 34 digits before the point", loc);
    tv_u128 m = x->len == 1 ? x->d[0] : (((tv_u128)x->d[1] << 64) | x->d[0]);
    if (m >= tv_pow10_u128(TV_DEC_DIGITS)) tv_trap("decimal overflow: more than 34 digits before the point", loc);
    return tv_dec_make(x->neg ? -(tv_i128)m : (tv_i128)m, 0);
}

/* int(d): traps unless it's an integer that fits. */
tv_int tv_dec_to_int(tv_dec a, const char *loc) {
    tv_dec t = tv_dec_trim(a, 0);
    if (tv_dec_scale(t) != 0) tv_trap("decimal is not an integer (round it first)", loc);
    tv_i128 c = tv_dec_coef(t);
    if (c < INT64_MIN || c > INT64_MAX) tv_trap("decimal out of range for an integer", loc);
    return (tv_int)c;
}

/* ---------------------------------------------------------------- the descriptor */

static bool tv_decp_eq(const void *a, const void *b) { return tv_dec_eq(*(const tv_dec *)a, *(const tv_dec *)b); }
static uint64_t tv_decp_hash(const void *p) { return tv_dec_hash(*(const tv_dec *)p); }
static void tv_decp_to_str(tv_sb *sb, const void *p) { tv_sb_push_dec(sb, *(const tv_dec *)p); }
static void tv_decp_inspect(tv_sb *sb, const void *p, int depth) {
    (void)depth;
    tv_sb_push_dec(sb, *(const tv_dec *)p);
}
const tv_type tv_type_dec = { sizeof(tv_dec), NULL, NULL, tv_decp_eq, tv_decp_hash, tv_decp_to_str, tv_decp_inspect };
