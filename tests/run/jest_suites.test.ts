function add(a: number, b: number) { return a + b; }
describe("math", () => {
  let n = 0;
  beforeEach(() => { n = 10; });
  it("adds", () => { expect(add(1, 2)).toBe(3); n++; });
  it("sees the hook", () => { expect(n).toBe(10); });
  describe("nested", () => {
    afterEach(() => { n = -1; });
    test("fails", () => { expect(add(1, 2)).toBe(4); });
    test("throws", () => { throw new Error("boom"); });
    it.skip("skipped", () => { expect(1).toBe(2); });
  });
});
it("top level it", () => { expect(add(2, 2)).toBe(4); });
