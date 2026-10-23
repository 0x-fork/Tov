// The test API imported from "@jest/globals" (or "vitest", "bun:test") is the built-in one; a
// statement under `// @ts-expect-error` that doesn't check throws a TypeError, as the input
// checks it gets past would.
import { describe, expect, it } from "@jest/globals";

function double(n: number): number {
  if (typeof n !== "number") throw new TypeError("not a number");
  return n * 2;
}

describe("double", () => {
  it("doubles", () => {
    expect(double(2)).toBe(4);
  });
  it("throws for a string", () => {
    expect(() => {
      // @ts-expect-error - checked at run time
      double("2");
    }).toThrow();
  });
});
