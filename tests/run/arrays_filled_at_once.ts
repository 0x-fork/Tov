// `Array(n)` filled as it's made: no hole is ever seen, so any element type will do.

const parents = Array(4).fill(null);
parents[2] = 7;
console.log(parents);

let k = 0;
const squares = Array(5)
  .fill(undefined)
  .map(() => {
    k++;
    return k * k;
  });
console.log(squares);

const names = new Array<string | null>(2).fill(null);
names[0] = "a";
console.log(names);
