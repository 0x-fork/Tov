// Strings that may be undefined or null (held as a string with no buffer).
class P {
  current: string | null = null;
  name: string | undefined;
  idx = 0;
  input = "ab\ncd";
  read(): void {
    this.idx += 1;
    this.current = this.idx < this.input.length ? this.input.substring(this.idx, this.idx + 1) : null;
  }
}
const p = new P();
console.log(p.current, p.name, p.current === null, p.name === undefined);
p.read();
console.log(p.current, p.current === "b", p.current !== "\n", JSON.stringify(p));
p.read();
console.log(p.current === "\n", String(p.current).length);
p.name = "zed";
console.log(`${p.name}!`, p.name?.toUpperCase(), p.name ?? "none", p.name.length);
const xs: (string | undefined)[] = ["a", undefined, "c"];
console.log(xs, xs.map((x) => x ?? "-").join(""), xs.indexOf(undefined), xs.includes("c"));
const m = new Map<string | null, number>();
m.set("k", 1); m.set(null, 2); m.set("k" + "", 3);
console.log(m.size, m.get("k"), m.get(null), [...m.keys()]);
const s = new Set<string | undefined>(["x", undefined, "x"]);
console.log(s.size, s.has(undefined), s.has("x"));
function pick(a: string | null, b: string | null): string { return a === b ? "same" : (a ?? "") + (b ?? ""); }
console.log(pick("q", "q"), pick(null, null), pick("q", null), pick("ab".slice(0, 1), "a"));
let opt: string | undefined;
for (const w of ["one", "two"]) opt = (opt ?? "") + w;
console.log(opt, opt?.length, typeof opt, typeof p.current);
const arr = [p.current, p.name];
console.log(arr, structuredClone(arr));
switch (p.current) {
  case "\n": console.log("newline"); break;
  case null: console.log("null"); break;
  default: console.log("other");
}
