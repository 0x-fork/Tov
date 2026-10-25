// `var`: one variable of the whole function, as JavaScript's (seen after its block, shared by
// closures made in a loop, the same as a parameter of its name).
function f(): number[] {
  const fs: (() => number)[] = [];
  for (let i = 0; i < 3; i++) {
    var x = i * 10;
    fs.push(() => x);
  }
  return fs.map((g) => g());
}
function g(n: number): number {
  if (n > 0) {
    var y = n * 2;
  } else {
    var y = -1;
  }
  return y;
}
function h(): number {
  for (var i = 0; i < 5; i++) {}
  return i;
}
function k(a: number): number {
  if (a > 1) { var a = 100; }
  return a;
}
function sum(xs: number[]): number {
  var t = 0;
  for (var j = 0; j < xs.length; j++) t += xs[j];
  return t + j;
}
const arrowV = () => { { var q = "in"; } return q; };
console.log(f(), g(3), g(-2), h(), k(5), k(0), sum([1, 2, 3]), arrowV());
