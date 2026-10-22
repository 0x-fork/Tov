function parse(s: string): number {
  if (s === "") throw new Error("empty");
  return Number(s);
}
describe("matchers", () => {
  it("passes", () => {
    expect([1, 2]).toStrictEqual([1, 2]);
    expect(null as string | null).toBeNull();
    expect(1).toBeTruthy();
    expect("").toBeFalsy();
    expect(0 / 0).toBeNaN();
    expect(5).toBeGreaterThan(3);
    expect(5).toBeLessThanOrEqual(5);
    expect([1, 2, 3]).toHaveLength(3);
    expect("hello").toHaveLength(5);
    expect([1, 2, 3]).toContain(2);
    expect("hello").toContain("ell");
    expect(() => parse("")).toThrow();
    expect(() => parse("")).toThrowError("empty");
    expect(() => parse("4")).not.toThrow();
    expect(3).not.toBe(4);
    expect([1]).not.toEqual([2]);
    expect(10n).toBeGreaterThan(3n);
  });
  it("fails not", () => { expect(3).not.toBe(3); });
  it("fails throw", () => { expect(() => parse("1")).toThrow(); });
  it("fails contain", () => { expect([1, 2]).toContain(5); });
  it("fails gt", () => { expect(1).toBeGreaterThan(2); });
  it("fails null", () => { expect("x" as string | null).toBeNull(); });
});
