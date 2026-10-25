// `(...args: A) => R` wrappers, `A` a tuple each call gives, and closures calling the local
// they initialize.
function memoize<A extends unknown[], R>(f: (...args: A) => R, key: (...args: A) => string): (...args: A) => R {
  const cache = new Map<string, R>();
  return (...args: A) => {
    const k = key(...args);
    let v = cache.get(k);
    if (v === undefined) {
      v = f(...args);
      cache.set(k, v);
    }
    return v;
  };
}

function logged<A extends unknown[], R>(name: string, f: (...args: A) => R): (...args: A) => R {
  return (...args: A) => {
    const r = f(...args);
    console.log(name, "->", r);
    return r;
  };
}

const fib: (n: number) => number = memoize((n: number): number => (n < 2 ? n : fib(n - 1) + fib(n - 2)), (n) => String(n));
console.log(fib(70));

const rep = memoize((s: string, n: number) => s.repeat(n), (s, n) => `${s}|${n}`);
console.log(rep("ab", 3), rep("ab", 3), rep("x", 0) === "");

const join3 = logged("join3", (a: string, b: number, c: boolean) => `${a}${b}${c}`);
join3("q", 7, false);
const now = logged("const", () => 42);
console.log(now() + 1);

const t: [number, string] = [2, "z"];
const both = (n: number, s: string) => s.repeat(n);
console.log(both(...t));

for (let r = 1; r <= 2; r++) {
  const words = ["kitten", "sitting"];
  const lev: (i: number, j: number) => number = memoize(
    (i: number, j: number): number => {
      if (i === 0) return j;
      if (j === 0) return i;
      const c = words[0][i - 1] === words[1][j - 1] ? 0 : r;
      return Math.min(lev(i - 1, j) + 1, lev(i, j - 1) + 1, lev(i - 1, j - 1) + c);
    },
    (i, j) => i + ":" + j,
  );
  console.log(r, lev(words[0].length, words[1].length));
  const down: (n: number) => number = (n) => (n <= 0 ? 0 : 1 + down(n - 1));
  console.log(down(5 * r));
}
