// `new Map()` / `new Set()` without type arguments: their keys and values are what they're
// given (`m.set(k, v)`, `s.add(x)`, or where they're used), as TypeScript's `Map<any, any>`
// holds them.

function minSwaps(input: number[]): number {
  const sorted = input.slice().sort((a, b) => a - b);
  const indexOf = new Map();
  for (let i = 0; i < input.length; i++) indexOf.set(input[i], i);
  const seen = new Set();
  let swaps = 0;
  for (let i = 0; i < input.length; i++) {
    if (seen.has(i) || indexOf.get(sorted[i]) === i) continue;
    let j = i;
    let cycle = 0;
    while (!seen.has(j)) {
      seen.add(j);
      j = indexOf.get(sorted[j]);
      cycle++;
    }
    if (cycle > 0) swaps += cycle - 1;
  }
  return swaps;
}

function wordCounts(text: string): Map<string, number> {
  const counts = new Map();
  for (const w of text.split(" ")) counts.set(w, (counts.get(w) ?? 0) + 1);
  return counts;
}

const names = new Set();
names.add("ada");
names.add("grace");
console.log(minSwaps([4, 3, 2, 1]), minSwaps([1, 5, 4, 3, 2]), wordCounts("a b a c b a"), names, names.size);
