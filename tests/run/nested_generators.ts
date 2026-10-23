// Generators and generic functions inside a function, using none of its variables: module
// functions of their own (a prime sieve of chained generators, as TheAlgorithms writes it).

export function* primes(): Generator<number> {
  function* filter(input: Generator<number>, prime: number): Generator<number> {
    while (true) {
      const next = input.next();
      if (next.done) break;
      if (next.value! % prime !== 0) yield next.value!;
    }
  }
  let chain: Generator<number> = (function* () {
    let i = 2;
    while (true) yield i++;
  })();
  while (true) {
    const next = chain.next();
    if (next.done) break;
    const p = next.value!;
    yield p;
    chain = filter(chain, p);
  }
}

function sortedBy(words: string[]): string[] {
  function by<T>(xs: T[], key: (x: T) => number): T[] {
    return [...xs].sort((a, b) => key(a) - key(b));
  }
  return by(words, (w) => w.length);
}

const ps: number[] = [];
for (const p of primes()) {
  if (p > 30) break;
  ps.push(p);
}
console.log(ps, sortedBy(["ccc", "a", "bb"]));
