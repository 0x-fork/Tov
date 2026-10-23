// Tests declared in functions a suite calls are named inside the suites running, as Jest's.

const square = (n: number): number => n * n;
const twice = (n: number): number => n + n;

const checkTable = (f: (n: number) => number, expected: number[]) =>
  it.each([
    [1, expected[0]],
    [3, expected[1]],
  ])("f(%i) = %i", (n, want) => expect(f(n)).toBe(want));

function checkStack(label: string) {
  describe(`${label} stack`, () => {
    let items: number[] = [];
    beforeEach(() => {
      items = [1, 2];
    });
    it("starts with two", () => {
      expect(items.length).toBe(2);
    });
    it("pushes", () => {
      items.push(3);
      expect(items).toEqual([1, 2, 3]);
    });
  });
  test("is labelled", () => {
    expect(label.length).toBeGreaterThan(0);
  });
}

describe("square", () => checkTable(square, [1, 9]));
describe("twice", () => {
  checkTable(twice, [2, 7]);
  describe("nested", () => checkStack("inner"));
});
checkStack("top");
