// A `let` given no value is read only once something is assigned to it, on every path.

function pick(c: boolean): number {
  let x: number;
  if (c) x = 1; else x = 2;
  return x;
}

function firstOver(n: number): number {
  let y: number;
  while (true) {
    if (n > 2) {
      y = n;
      break;
    }
    n++;
  }
  return y;
}

function code(s: string): number {
  let z: number;
  switch (s) {
    case "a":
      z = 1;
      break;
    case "b":
      z = 2;
      break;
    default:
      throw new Error("no code for " + s);
  }
  return z;
}

function attempt(): number {
  let w: number;
  try {
    w = 3;
  } catch {
    w = 4;
  }
  let r: number;
  do {
    r = 7;
  } while (r < 3);
  let v: number | undefined;
  return w + r + (v ?? 0);
}

function swap(): string {
  let a: string, b: string;
  [a, b] = ["x", "y"];
  return b + a;
}

let greeting: string;
greeting = "hi";

console.log(pick(true), pick(false), firstOver(0), code("b"), attempt(), swap(), greeting);
