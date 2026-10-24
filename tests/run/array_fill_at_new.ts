// `new Array<T>(n).fill(v)`: made with n copies of v at once.
class Point {
  constructor(public x: number) {}
}
const zeros = new Array<number>(4).fill(0);
const negs = new Array<number>(3).fill(-0);
console.log(zeros.join(","), negs.map((x) => Object.is(x, -0)).join(","));
const p = new Point(1);
const ps = new Array<Point>(3).fill(p);
ps[0].x = 7;
console.log(ps.map((q) => q.x).join(","), ps[1] === ps[2], p.x);
const strs = new Array<string>(2).fill("ab");
console.log(strs.join("|"), new Array<boolean>(3).fill(true).join(","), new Array<number>(0).fill(5).length);
const n = 2.0;
const half = new Array<number>(n * 2).fill(0.5);
console.log(half.join(","), half.length);
type Kind = "a" | "b" | "c";
const kinds = new Array<Kind>(3).fill("b");

console.log(kinds.join(""));
const grid = new Array<number[]>(2).fill([]);
grid[0].push(1);
console.log(grid[1].length, grid[0] === grid[1]);
