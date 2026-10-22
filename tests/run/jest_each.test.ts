function isPow2(n: number): boolean { return n > 0 && (n & (n - 1)) === 0; }
function sum(xs: number[]): number { return xs.reduce((a, b) => a + b, 0); }
describe("each", () => {
  it.each([
    [0, false],
    [1, true],
    [4, true],
  ])("isPow2(%i) is %s", (n, expected) => {
    expect(isPow2(n)).toBe(expected);
  });
  test.each([
    { input: [1, 2], expected: 3 },
    { input: [], expected: 0 },
  ])("sum of $input is $expected", ({ input, expected }) => {
    expect(sum(input)).toBe(expected);
  });
  const cases: number[][] = [[3, 1, 2], [5, 4]];
  test.each(cases)("case %#", (...arr: number[]) => {
    expect([...arr].sort((a, b) => a - b)[0]).toBeLessThanOrEqual(arr[0]);
  });
});
test.each([[2, 3, 5], [1, 1, 2]])("%i + %i = %i", (a, b, c) => { expect(a + b).toBe(c); });
