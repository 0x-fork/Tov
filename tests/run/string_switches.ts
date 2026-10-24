// `switch` over a string with literal cases: matched first (on its byte, when every case is
// one ASCII character), the value borrowed for that.
function kind(c: string | null): string {
  switch (c) {
    case "a":
    case "b":
      return "ab";
    case "a":
      return "never";
    default:
      return "other";
    case "z":
      return "z";
    case "\n":
      return "newline";
  }
}
function word(w: string): number {
  let n = 0;
  switch (w) {
    case "one":
      n += 1;
      break;
    case "two":
      n += 2;
      break;
    case "é":
      n = 100;
      break;
    case "":
      n = -1;
  }
  return n;
}
class Reader {
  text = "ab\nz?";
  i = 0;
  cur: string | null = "a";
  next(): string {
    // the switched-over field changes in the case it picks
    switch (this.cur) {
      case "a":
        this.cur = "q";
        return "was a";
      default:
        this.cur = null;
        return "was other";
    }
  }
}
const out: string[] = [];
for (const c of ["a", "b", "z", "\n", "?", "ab"]) out.push(kind(c));
out.push(kind(null));
console.log(out.join(","));
console.log(["one", "two", "three", "é", ""].map(word).join(","));
const r = new Reader();
console.log(r.next(), r.next(), r.next(), r.cur);
const parts = "x,y".split(",");
switch (parts.join("")) {
  case "xy":
    console.log("joined");
    break;
  default:
    console.log("no");
}
