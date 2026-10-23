// `xs[i] === x` past either end of a number array is false (`!==` true), as JavaScript's
// `undefined === x` is.

function search(array: number[], target: number): number | null {
  let offset = -1;
  const b = 1;
  if (b && array[offset + 1] === target) return offset + 1;
  return null;
}
const xs = [3, 4];
let i = 5;
console.log(search([], 1), search([1], 1), xs[i] === 4, xs[i] !== 4, xs[-1] == 3, xs[1] === 4);
