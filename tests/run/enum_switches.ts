// An enum's members are constants: a switch over them is a C switch. An object constant that is
// written to, or handed on, is read as an object.
enum Op { Push, Pop = 5, Neg = -2, Next }
const enum Flag { A = 1, B = 2, C = 4 }
function name(op: number): string {
  switch (op) {
    case Op.Push:
      return "push";
    case Op.Pop:
    case 7:
      return "pop/7";
    default:
      return "other";
    case Op.Neg:
      return "neg";
    case Op.Next:
      return "next";
    case 5:
      return "never";
  }
}
console.log([0, 5, 7, -2, -1, 6, 9].map(name).join(","));
function bits(f: number): number {
  let n = 0;
  switch (f & 3) {
    case Flag.A:
      n += 10;
      break;
    case Flag.B:
      n += 20;
      break;
    case 0:
      break;
    default:
      n = -1;
  }
  return n + (f & Flag.C);
}
console.log(bits(1), bits(2), bits(3), bits(4), bits(7));
// an object constant written to: its fields read from it
const limits = { low: 1, high: 9 };
function bump(): void { limits.high += 1; }
bump();
switch (10) {
  case limits.high:
    console.log("high is 10");
    break;
  default:
    console.log("high is", limits.high);
}
const shared = { k: 3 };
function show(o: { k: number }): number { return o.k * 2; }
console.log(show(shared), shared.k);
enum Color { Red = "red", Green = "green" }
const c: Color = Color.Green;
switch (c) {
  case Color.Red: console.log("r"); break;
  case Color.Green: console.log("g"); break;
}
