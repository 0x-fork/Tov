type Result<T, E = string> = { ok: true; value: T } | { ok: false; error: E };
function parseNum(s: string): Result<number> {
  const n = Number(s);
  return Number.isNaN(n) ? { ok: false, error: `bad number: ${s}` } : { ok: true, value: n };
}
for (const s of ["42", "x", "3.5"]) {
  const r = parseNum(s);
  if (r.ok) console.log("value", r.value * 2);
  else console.log("error", r.error);
}
function unwrap<T>(r: Result<T>, d: T): T {
  if (!r.ok) return d;
  return r.value;
}
console.log(unwrap(parseNum("7"), 0), unwrap(parseNum("?"), -1));
const results: Result<string>[] = [{ ok: true, value: "a" }, { ok: false, error: "nope" }];
console.log(results.filter((r) => r.ok).length, results.map((r) => (r.ok === false ? r.error : r.value)));
console.log(JSON.stringify(results[0]), results[1]);
type Dice = 1 | 2 | 3 | 4 | 5 | 6;
const roll: Dice = 4;
const flags: { done: true } = { done: true };
console.log(roll + 1, flags.done, flags);
type Shape = { kind: "circle"; r: number } | { kind: "sq"; s: number };
function area(s: Shape): number {
  switch (s.kind) {
    case "circle":
      return 3 * s.r * s.r;
    case "sq":
      return s.s * s.s;
    default: {
      const _exhaustive: never = s;
      return _exhaustive;
    }
  }
}
console.log(area({ kind: "circle", r: 2 }), area({ kind: "sq", s: 3 }));
