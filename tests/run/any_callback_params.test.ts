// `any` on a callback's parameters, where what's passed is known (`test.each`'s rows): those
// types, as if they were unannotated.

const find = (xs: number[], x: number): number => xs.indexOf(x);

describe("find", () => {
  test.each([
    [[1, 3, 5], 3, 1],
    [[2, 4], 5, -1],
  ])("of %o, searching for %o, expected %i", (array: any[], target: any, index: number) => {
    expect(find(array, target)).toStrictEqual(index);
  });
});

test("map with any", () => {
  const doubled = [1, 2, 3].map((x: any) => x * 2);
  expect(doubled).toEqual([2, 4, 6]);
});
