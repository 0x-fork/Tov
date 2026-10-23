// A `test.each` table made by code, `xs.map((x) => [x])`: an array row is spread over the
// test's parameters, as Jest spreads it.

const log2 = (n: number): number => Math.floor(Math.log2(n));
test.each([...Array(4).keys()].map((i) => [2 ** i]))("log2(%i)", (input) => {
  expect(log2(input)).toBe(Math.round(Math.log2(input)));
});
test.each(["a", "bb"].map((s) => ({ s, n: s.length })))("$s has $n", ({ s, n }) => {
  expect(s.length).toBe(n);
});
