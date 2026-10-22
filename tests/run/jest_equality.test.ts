test("objects", () => {
  expect(NaN).toBe(NaN);
  expect(0 / 0).toEqual(NaN);
  expect(new Set([1, 4, 2])).toEqual(new Set([1, 2, 4]));
  expect(new Map([[1, "a"], [2, "b"]])).toEqual(new Map([[2, "b"], [1, "a"]]));
  expect(-7 % 2).toBe(-1);
  expect(-4 % 2).toBe(-0);
  expect(5.5 % 2).toBe(1.5);
  expect(1e300 % 7).toBe(1e300 % 7);
});
test("not equal sets", () => { expect(new Set([1, 2])).toEqual(new Set([1, 3])); });
