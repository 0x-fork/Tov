const fns: (() => number)[] = [];
for (let i = 0; i < 3; i++) {
  var x = i * 10;
  fns.push(() => x);
}
console.log(fns.map((f) => f()).join(","), x);
for (var k = 0; k < 3; k++) {}
console.log(k);
if (true) { var y = "y"; }
console.log(y);
function f() { for (var j = 0; j < 2; j++) { var z = j; } return z + j; }
console.log(f());
