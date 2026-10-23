// Tests declared in a suite's loops and `if`s are its tests, declared as the statement runs.

const first = (xs: number[], x: number): number | null => {
  const i = xs.indexOf(x);
  return i < 0 ? null : i;
};
const last = (xs: number[], x: number, from = xs.length - 1): number | null => {
  for (let i = from; i >= 0; i--) if (xs[i] === x) return i;
  return null;
};
type Find = { (xs: number[], x: number): number | null };
const finds: Find[] = [first, last];

describe("finds", () => {
  for (const f of finds) {
    it("is defined", () => {
      expect(f([1, 2], 2)).toBeDefined();
    });
    it("finds 2", () => {
      expect(f([1, 2, 3], 2)).toBe(1);
    });
    if (f === last) it("misses 9", () => expect(f([1], 9)).toBe(null));
  }
});
