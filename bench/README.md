# Tov micro-benchmarks

Small programs written four ways (C, Rust, TypeScript, Tov) that implement the same algorithm on the same data and print identical stdout. The runner builds each one, times it, measures peak memory, and checks that the output matches C's.

Languages: `c`, `rust`, `node` and `bun` (both run `main.ts` directly), `scriptc` (Vercel Labs' TS→native compiler, run on the same `main.ts`), and `tov`.

## Benchmarks

| Benchmark | Size | What it exercises |
|---|---|---|
| `fib` | `fib(42)` | Recursive calls, integer add/compare. Tov `int` is 64-bit with overflow traps; JS uses f64 numbers (SMIs in the JIT). |
| `points` | 600M iterations | Small record churn: `v = add(v, one)` returns a fresh `{x, y}` f64 record every iteration. In C/Rust it's a struct in registers; in JS it depends on escape analysis; in Tov it's a value record. |
| `array_push` | 30M `f64` | Growable-array `push` (amortised growth), then a `for…of` sum and an indexed `xs[i]!` sum. |
| `map_insert` | 2M keys | String building (`"k" + i`), hashing, and a string → int map: insert all keys, then look every key up again and sum. |
| `binary_trees` | depth 18 | Benchmarks Game binary-trees (single-threaded): many short-lived recursive allocations plus one long-lived tree. Tov uses a recursive record `{ left: Tree \| undefined, right: Tree \| undefined }`; Rust `Option<Box<Tree>>`; C `malloc`/`free`; TS plain objects. |
| `class_trees` | depth 18 | Binary-trees again, with a class (`new Tree(left, right)`, `t.check()` method) instead of a record: object allocation, reference counting, method calls. Rust `Box`, C `malloc`, TS classes. |
| `dispatch` | 50M calls | An array of 1000 shapes of three classes; `area()` called through the base type. Rust `Box<dyn Shape>`, C function-pointer vtables, TS classes. |
| `nbody` | 10M steps | Benchmarks Game n-body (5 bodies): f64 arithmetic, `sqrt`, in-place mutation of records inside an array (`inout` in Tov). Prints energy before and after to 9 decimals. |
| `fannkuch_redux` | n = 11 | Benchmarks Game fannkuch-redux (single-threaded): every permutation of 11 items, counting prefix reversals. Small `int[]` arrays indexed by values read from them (bounds checks that can't be hoisted), integer compare and swap. Prints the checksum and `Pfannkuchen(11) = 51`. |
| `spectral_norm` | n = 5,500 | Benchmarks Game spectral-norm (single-threaded): 10 power-iteration steps of `AᵀA·u` with `A[i][j] = 1 / ((i+j)(i+j+1)/2 + i + 1)`, about 1.2 billion f64 divisions over `f64[]` arrays. In TS and Tov `/` makes the denominator f64; C and Rust compute it in integers. Prints the norm to 9 decimals. |
| `mandelbrot` | 4000×4000 | Benchmarks Game mandelbrot (single-threaded): a tight f64 loop of up to 50 iterations per pixel, bits packed 8 to a byte. Prints the size, byte count and a checksum of the bitmap instead of the PBM. The C turns off `a*b+c` fusing (`#pragma STDC FP_CONTRACT OFF`), or pixels on the set's edge come out differently. |
| `fasta` | n = 10M | Benchmarks Game fasta (single-threaded): the ALU repeat, then two sequences drawn with the LCG and a linear scan of cumulative probabilities, in 60-column lines. Each line goes to a sink that counts lines and bytes and checksums them (`h = (h * 31 + b) & 0xffffff`) rather than to stdout. Module-level `let` state in TS/Tov, a static seed and a `Sink` struct in C/Rust. |
| `knucleotide` | 5M bases | Benchmarks Game k-nucleotide (single-threaded) on a sequence generated in the program with fasta's LCG and Homo sapiens frequencies: k-mer counts for k = 1, 2 (sorted percentages) and for GGT … GGTATTTTAATTTATAGT, in a string-keyed hash map. C uses a hand-written open-addressing table whose keys point into the sequence, Rust std `HashMap<&[u8], i64>` (borrowed keys, SipHash, `entry`), TS and Tov `Map<string, …>` with `seq.slice(i, i + k)` keys and `set(key, (get(key) ?? 0) + 1)`. |
| `sort` | 3M `f64` | Park–Miller LCG (`state = state * 16807 % 2147483647`, exact in both 64-bit ints and JS doubles), then sort with a comparator. Prints min, max and the sum of every 1000th element. |
| `loops` | 10,000 × 100,000 | The "1 billion nested loop iterations" program from [bddicken/languages](https://github.com/bddicken/languages): `a[i] += j % u` into a 10,000-element int array, then `a[i] += r`. `u` is 40 plus the argument count and `r` comes from a Park–Miller LCG seeded with it, so neither can be constant-folded. A runtime integer `%` per iteration. C and Rust use 32-bit ints as the original does; Tov `int` is 64-bit and overflow-checked. |
| `levenshtein` | 800 words, 319,600 pairs | Levenshtein distance (two-row DP, byte comparisons, the shorter string on the inner loop) between every pair of 800 words of 20–60 letters over an 8-letter alphabet, generated with a Park–Miller LCG. Prints the minimum and the sum. Tov and TS take the bytes with `new TextEncoder().encode(s)` once per word; C uses `char *`, Rust `as_bytes()`. |
| `strings` | 3M strings | Template literals / formatting, `join(",")`, `split(",")`, `endsWith`, and byte lengths. |
| `json` | 200k records × 5 | `JSON.stringify` an array of records, then parse it back typed (Tov validates the shape; Rust uses serde and serde_json). |
| `async_calls` | 30M awaits | `total = await step(total, i)`: calling an async function and awaiting its result. Rust uses tokio's current-thread runtime. |
| `async_tasks` | 1,000 × 10,000 | 1,000 async tasks each yielding to the event loop 10,000 times (`await Promise.resolve(0)`; Rust: `tokio::task::yield_now()` on the current-thread runtime), then `Promise.all`. |
| `bigint_pidigits` | 10,000 digits | Benchmarks Game pidigits: a spigot over bigints that grow to ~7,500 limbs (multiply by a small number, add, divide with a one-digit quotient, compare). Rust uses num-bigint, the most used Rust bigint crate (`bigint` below compares others). |
| `bigint_fib` | fib(300,000) | Repeated bigint addition up to 62,700 digits, then the decimal string. |
| `decimal_ledger` | 2,000,000 charges | Amounts parsed from text ("1234.56"), a 2.9% + 0.30 fee rounded to cents half to even, net per merchant, payouts at an FX rate. Rust uses rust_decimal; Node and Bun use big.js (the most used decimal library for JavaScript). |
| `decimal_invoice` | 200,000 invoices | Lines of price × quantity, a lump-sum discount allocated across each invoice's lines in proportion (a division per line, rounded to cents), 8.25% tax per line. rust_decimal and big.js again. |
| `bigint_factorial` | 40,000! | `f *= BigInt(i)` up to 166,714 digits, then the decimal string, its digit sum and trailing zeros. |

Each `bench/micro/<name>/` holds `main.c`, `main.rs`, `main.ts`, `main.tov` and `expected.txt` (C's output, used as the reference when C isn't among the selected languages). Where idiomatic Rust needs crates (tokio, serde), the Rust is a Cargo project in `rust/` instead of `main.rs`, and there's no C. Sizes are chosen so the C version takes roughly 0.2–1.5 s on an Apple M-series machine. The exception is `array_push`, where C takes about 55 ms. The work is memory-bound, and reaching 200 ms would need about 1 GB of array in C and about 3 GB of heap in Node/Bun.

## Running

```sh
cargo build --release                                  # for tov check / tov build
npm install --prefix bench/.tools scriptc              # optional: scriptc
python3 bench/run.py                                   # everything, 5 timed runs each
python3 bench/run.py --only fib,sort --langs c,tov    # a subset
python3 bench/run.py --runs 10 --warmup 1 --timeout 120
python3 bench/run.py --list
```

Python 3.9+ with only the standard library. Output is a markdown summary (median time, ratio against C), a detail table (median/min time, peak RSS, compile time, binary size unstripped and stripped, output check), and a list of anything not measured and why. Each run also writes `bench/results/<timestamp>.json` with every sample, the toolchain versions, and the machine's load average.

How each language is built and run:

| Lang | Build | Run |
|---|---|---|
| c | `cc -std=c11 -O2 main.c -lm` | the binary |
| rust | `rustc --edition 2021 -C opt-level=3 main.rs` (no Cargo, no deps); a `rust/` Cargo project with `cargo build --release` (LTO, one codegen unit), its crates built first and not timed | the binary |
| rust-checked | as `rust`, plus `-C overflow-checks=on` (Tov's default integer semantics) | the binary |
| node | — | `node main.ts` (Node ≥ 23.6 strips types natively) |
| bun | — | `bun main.ts` |
| scriptc | `bench/.tools/node_modules/.bin/scriptc build main.ts -o … --no-keep-c`, with `SCRIPTC_NO_CACHE=1` so compile time is a clean build (`--scriptc-cache` to allow cache hits) | the binary |
| tov | `target/release/tov build main.tov -o …` | the binary |
| tov-unchecked | as `tov`, plus `--unchecked` (integer overflow wraps, like Rust release builds) | the binary |

Tov builds use a cache private to each benchmark session, so every program is really compiled. The runtime object is built once at the start of the session (outside the timing), the way it is on any machine after the first build, much like Rust's precompiled standard library.

A missing toolchain is reported as `skipped`. A failed build is `build failed` with the compiler's message kept in the JSON; for scriptc a rejected program is `unsupported` (the TS isn't rewritten to suit it). A run past `--timeout` (default 60 s) is `timeout`, a non-zero exit is `crashed`, and output that differs from C is `mismatch`. Binaries go to `bench/.build/`.

## Results

Median wall time and peak memory of 5 runs (after a warmup run). Every language prints the same output. The JSON behind each table is in `bench/results/`.

### macOS, Apple M4 Max

Load average about 3. Apple clang 17, Rust 1.98, Node 26.3, Bun 1.4.0.

| benchmark | C | Rust | Node | Bun | **Tov** |
|---|---:|---:|---:|---:|---:|
| array_push | 51 ms · 244 MB | 59 ms · 244 MB | 347 ms · 780 MB | 207 ms · 814 MB | **48 ms** · 242 MB |
| async_calls | — | 14 ms · 2.0 MB | 643 ms · 81.1 MB | 525 ms · 29.9 MB | **17 ms** · 1.9 MB |
| async_tasks | — | 136 ms · 2.3 MB | 353 ms · 97.4 MB | 179 ms · 30.9 MB | **137 ms** · 2.5 MB |
| binary_trees | 686 ms · 18.7 MB | 793 ms · 19.0 MB | 461 ms · 212 MB | 381 ms · 195 MB | **260 ms** · 18.7 MB |
| class_trees | 690 ms · 18.7 MB | 793 ms · 19.0 MB | 467 ms · 210 MB | 371 ms · 223 MB | **259 ms** · 18.7 MB |
| dispatch | 76 ms · 1.9 MB | 76 ms · 2.1 MB | 201 ms · 81.9 MB | 109 ms · 26.7 MB | **39 ms** · 1.9 MB |
| fib | 492 ms · 1.9 MB | 439 ms · 2.0 MB | 1500 ms · 78.1 MB | 948 ms · 18.8 MB | **427 ms** · 1.9 MB |
| json | — | 260 ms · 241 MB | 444 ms · 491 MB | 267 ms · 327 MB | **226 ms** · 143 MB |
| map_insert | 524 ms · 235 MB | 534 ms · 311 MB | 945 ms · 295 MB | 764 ms · 592 MB | **276 ms** · 157 MB |
| nbody | 310 ms · 1.8 MB | 221 ms · 2.0 MB | 556 ms · 80.9 MB | 505 ms · 23.1 MB | **206 ms** · 1.9 MB |
| points | 288 ms · 1.9 MB | 299 ms · 2.0 MB | 1891 ms · 86.7 MB | 815 ms · 32.0 MB | **288 ms** · 1.8 MB |
| sort | 268 ms · 25.9 MB | 73 ms · 40.3 MB | 1185 ms · 279 MB | 711 ms · 121 MB | **47 ms** · 26.2 MB |
| strings | 254 ms · 216 MB | 230 ms · 263 MB | 517 ms · 701 MB | 208 ms · 592 MB | **155 ms** · 192 MB |

### bigint (macOS, Apple M4 Max)

`python3 bench/run.py --only bigint_pidigits,bigint_fib,bigint_factorial --langs rust,node,bun,tov`, 5 runs ([results/20261011-221701.json](results/20261011-221701.json)):

| benchmark | Rust (num-bigint) | Node | Bun | **Tov** |
|---|---:|---:|---:|---:|
| bigint_pidigits | 3117 ms · 6.6 MB | 5442 ms · 140 MB | 747 ms · 100 MB | **289 ms · 4.4 MB** |
| bigint_fib | 501 ms · 4.5 MB | 456 ms · 105 MB | 366 ms · 102 MB | **168 ms · 3.1 MB** |
| bigint_factorial | 94 ms · 5.0 MB | 224 ms · 90 MB | 544 ms · 96 MB | **64 ms · 4.4 MB** |

pidigits against Rust's other bigint crates, the same program on each (best of 15 interleaved runs, a loaded machine): GMP through `rug` 284 ms, Tov 289, malachite 707, dashu 760, ibig 1,129, num-bigint 3,117. GMP is hand-written assembly; Tov's limb loops are C.

What makes them fast: a bigint below 2^62 is one tagged word (no allocation); larger ones are reference-counted buffers that `x op= y` (and `x = (x + a * b) * c`) update in place when nothing else holds them; `z + x * y` is one pass; the limb loops take eight limbs a step along one add-with-carry chain; division estimates each quotient digit from a precomputed reciprocal (and skips the check pass when the estimate can't be off); decimal output divides and conquers.

### decimal (macOS, Apple M4 Max)

`python3 bench/run.py --only decimal_ledger,decimal_invoice --langs rust,node,bun,tov`, 5 runs ([results/20261014-002611.json](results/20261014-002611.json)). Every implementation prints the same totals to the cent.

| benchmark | Rust (rust_decimal) | Node (big.js) | Bun (big.js) | **Tov** |
|---|---:|---:|---:|---:|
| decimal_ledger | 143 ms · 1.9 MB | 1984 ms · 182 MB | 1866 ms · 67 MB | **109 ms · 1.9 MB** |
| decimal_invoice | 132 ms · 1.9 MB | 8973 ms · 159 MB | 6550 ms · 77 MB | **52 ms · 1.9 MB** |

A decimal is 128 bits: a 34-digit coefficient and its scale, so same-scale `+`, `-`, `<` are single 128-bit operations, and a product of two 64-bit coefficients one multiply. Division by a coefficient under 2^64 goes 19 digits at a time by reciprocal; by a power of ten it only changes the scale; `(a / b).round(k)` divides straight to k places when that provably gives what dividing then rounding gives (it falls back otherwise).

### Linux, x86-64 (AMD Ryzen 9 9950X, 8 vCPUs, a cloud VM)

Ubuntu 24.04, load average about 1. C with GCC 13, Tov's C with clang 18, Rust 1.97, Node 24.18, Bun 1.4.0. `map_insert`'s C, Rust and Tov are from a second run (its C didn't build on Linux the first time); the VM's timings vary by up to a third between runs.

| benchmark | C | Rust | Node | Bun | **Tov** |
|---|---:|---:|---:|---:|---:|
| array_push | 97 ms · 242 MB | 89 ms · 242 MB | 507 ms · 781 MB | 334 ms · 606 MB | **99 ms** · 242 MB |
| async_calls | — | 24 ms · 2.1 MB | 879 ms · 73.8 MB | 701 ms · 34.3 MB | **18 ms** · 1.8 MB |
| async_tasks | — | 309 ms · 2.6 MB | 524 ms · 104 MB | 226 ms · 35.5 MB | **132 ms** · 2.2 MB |
| binary_trees | 791 ms · 35.1 MB | 893 ms · 35.7 MB | 994 ms · 313 MB | 664 ms · 176 MB | **222 ms** · 18.6 MB |
| class_trees | 997 ms · 35.1 MB | 986 ms · 35.7 MB | 641 ms · 270 MB | 748 ms · 204 MB | **242 ms** · 18.6 MB |
| dispatch | 120 ms · 1.8 MB | 123 ms · 2.4 MB | 117 ms · 73.0 MB | 78 ms · 29.6 MB | **24 ms** · 1.8 MB |
| fib | 235 ms · 1.6 MB | 451 ms · 2.1 MB | 1572 ms · 68.9 MB | 1054 ms · 23.6 MB | **480 ms** · 1.8 MB |
| json | — | 370 ms · 138 MB | 663 ms · 453 MB | 439 ms · 308 MB | **248 ms** · 90.2 MB |
| map_insert | 633 ms · 194 MB | 1456 ms · 256 MB | 2035 ms · 331 MB | 1531 ms · 643 MB | **579 ms** · 101 MB |
| nbody | 358 ms · 1.7 MB | 261 ms · 2.2 MB | 484 ms · 72.7 MB | 666 ms · 27.3 MB | **318 ms** · 1.8 MB |
| points | 235 ms · 1.7 MB | 236 ms · 2.2 MB | 2116 ms · 73.6 MB | 826 ms · 37.2 MB | **230 ms** · 1.8 MB |
| sort | 320 ms · 49.3 MB | 96 ms · 38.1 MB | 2097 ms · 335 MB | 666 ms · 125 MB | **49 ms** · 25.8 MB |
| strings | 289 ms · 309 MB | 257 ms · 262 MB | 962 ms · 834 MB | 421 ms · 540 MB | **186 ms** · 158 MB |

Where Tov is behind:

- `async_calls` on macOS: Rust with tokio takes 14 ms and Tov 17. Tov checks every `+` for overflow and Rust's release builds don't; on Linux Tov is ahead (18 ms against 24).
- `nbody` on Linux: 318 ms against Rust's 261. On macOS Tov is ahead (206 against 221).
- `fib` on Linux: GCC turns C's recursion into a loop and runs it in 235 ms. Tov's overflow-checked additions can't be reordered that way; Rust takes 451 ms and Tov 480.
- Memory floor: a program that allocates little peaks at 1.8–1.9 MB, the same as C's on macOS and about 0.1–0.2 MB above it on Linux, and below Rust's (2.0–2.4 MB) on both.

## How the versions correspond

- The TS and Tov files are nearly line-for-line identical. They differ only where the spec requires it:
  - Tov uses `int` or `f64` where TS says `number`.
  - `main.ts` ends with a `main()` call, because Tov's entry point is `function main()` and top-level calls aren't allowed.
  - Strings use `.byteLength` in Tov and `.length` in TS (the strings are ASCII, so the values are equal).
  - In `nbody`, Tov passes the bodies array with `inout`/`&` and binds it with `let`.
- Indexed access is written `xs[i]!` in both. TS accepts it, and it type-checks under `tsc --strict --noUncheckedIndexedAccess`.
- `nbody` mutates elements through the array (`bodies[i]!.vx -= …`) rather than through a local alias. Under Tov's value semantics, `const bi = bodies[i]!` is a copy, so mutating the alias wouldn't write back. The same code is kept in TS.
- `binary_trees` reads `t.left`/`t.right` into locals before the `=== undefined` test. See the checker issue below.
- Floats are printed with `toFixed(n)` in TS/Tov, `%.nf` in C and `{:.n}` in Rust. All four round from the exact binary value, and no printed value is a tie.

## Caveats (read before quoting numbers)

- **Noise.** Wall time is measured for the whole process. The published results were collected on a busy developer machine (load average 8–17 on 16 cores, including a browser and other builds), Within one invocation the five runs usually agree to within a few percent, but the same binary can be 2–3× slower in a session where many cores are busy, and slow machine-wide phases can skew one language's column relative to another's. The JSON records the load average. For quotable numbers, use an idle machine and `--runs 10`, and look at `min` as well as the median.
- **Peak memory on Linux.** Linux starts a process's peak resident set at its parent's, so on Linux every program is run through a small launcher (`rusage.c`) rather than straight from Python, whose 15–20 MB would otherwise hide anything smaller.
- **Startup.** Node and Bun times include runtime startup (~20–40 ms) and JIT warm-up, and their RSS includes the VM. That's deliberate: it's what a user of a CLI sees.
- **Library quality differs:**
  - `map_insert`: C uses a small hand-written open-addressing table (FNV-1a, cached hashes), Rust uses std `HashMap` with SipHash (DoS-resistant and slower to hash), and JS uses the engine's `Map`.
  - `sort`: C's `qsort` is unstable and calls the comparator through a function pointer. Rust's `sort_by` is a stable driftsort. JS uses TimSort.
  - `strings`: Rust's `split` borrows `&str` slices without allocating; C and JS allocate every piece.
- **Semantics differ in the corners.** Tov `int` arithmetic traps on overflow, while C and Rust release builds don't check. Rust bounds-checks indexing and C doesn't. Tov `xs[i]!` traps.
- **Memory management differs in `binary_trees`.** C frees explicitly, Rust drops, and JS/Bun rely on a generational GC, which suits this workload well. This is the single-threaded variant. The fastest Benchmarks Game entries use threads and arenas, and none is used here.
- **Binary size** is reported both as linked and after `strip`. Rust binaries carry std symbols unless stripped.
- **Apple clang contracts `a*b+c` into FMA by default** (`-ffp-contract=on`), unlike Rust and JS. The printed `nbody` energies still match to 9 decimals, but the C and JS computations are not bit-identical.

## scriptc notes (v0.1.7)

- `nbody` is **unsupported**: `error SC1090: compound assignment to fields of computed receivers are not supported yet` on `bodies[i]!.vx -= dx * mj` (9 sites).
- `sort` **times out**. With a comparator, scriptc's sort takes 0.6 s for 1M elements but runs for more than 60 s at 2M. At 3M it was still running after 9 minutes, so something is super-linear. The program is correct at smaller sizes: 30k, 100k, 300k and 1M elements all match Node.
- `array_push` compiles and is correct but is about 100× slower than C. `points` (fresh object per iteration) and `binary_trees` are also 6–18× slower than C, because every object is `malloc`'d and reference-counted.

## Tov checker issues

Found while writing `main.tov` files against `docs/spec.md` with `tov check` at M0. The Tov files currently use the workarounds noted.

1. **No narrowing of property paths against `undefined`.** TypeScript narrows `t.left` after `t.left !== undefined`. The spec lists `x === undefined` / `x !== undefined` among the narrowing forms without saying whether `x` may be a property path. It does say "if code looks like TypeScript it behaves like TypeScript, or it is a compile error". The natural binary-trees `check`:

   ```ts
   function check(t: Tree): int {
     if (t.left === undefined || t.right === undefined) return 1
     return 1 + check(t.left) + check(t.right)
   }
   ```

   is rejected with:

   ```
   error[T0001] micro/binary_trees/main.tov:12:20: expected `Tree`, found `Tree | undefined`
      12 |   return 1 + check(t.left) + check(t.right)
     context: argument `t` of `check`
     why: the value may be `undefined`
     fix[placeholder]: provide a default: `?? <default>`
     fix[maybe]: assert it is defined (traps if not): `!`
   ```

   The same error appears for `t.right` (col 36), and for the simpler `if (t.left !== undefined) return 1 + check(t.left)`. The error has a code and fixes, so it's arguably spec-conforming. But it rejects idiomatic TS, and neither suggested fix is the clean rewrite: binding locals first (`const left = t.left`), which is what `main.tov` does. Either the spec should say that only local bindings narrow, or the checker should narrow `const`-rooted property paths. With value semantics nothing can mutate `t` between the test and the use, so the latter is sound.

2. **`inout` arguments must be `let` bindings (clarification, not a bug).** `const bodies = createBodies(); advance(&bodies, 0.01)` gives:

   ```
   error[V0001] micro/nbody/main.tov:127:19: can't reassign constant `bodies`
     127 |   offsetMomentum(&bodies)
     fix[safe]: declare `bodies` with `let`
   ```

   That's reasonable, since an `inout` callee may reassign the whole value. But spec §6 says `const` bindings "can be mutated in place", and its `addOne(&items)` example doesn't say `items` must be `let`. The spec should state the rule.

Everything else used here type-checks: recursive records, `Map<string, int>`, template literals, `join`/`split`/`endsWith`, `sort` with an `f64` comparator, `toFixed`, `1 << n`, `Math.max` on ints, field compound-assignment through `xs[i]!`, and top-level `const` with `Math.PI`.
