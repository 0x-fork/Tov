interface Para { type: "paragraph"; raw: string; tokens: Tok[] }
interface Br { type: "br"; raw: string }
interface Generic { [index: string]: any; type: string; raw: string; tokens?: Tok[] }
type Tok = Para | Br | Generic;

function count(ts: Tok[]): number {
  let n = ts.length;
  for (const t of ts) {
    switch (t.type) {
      case "paragraph":
        n += count(t.tokens ?? []);
        break;
      case "custom":
        n += 100 + t.raw.length;
        break;
      default:
        n += t.raw.length;
        break;
    }
  }
  return n;
}

function kind(t: Tok): string {
  switch (t.type) {
    case "br":
      return "break:" + t.raw.length;
    case "paragraph":
      return "para";
  }
  return "other:" + t.type;
}

const ts: Tok[] = [
  { type: "paragraph", raw: "p", tokens: [{ type: "br", raw: "\n" }] },
  { type: "custom", raw: "abc" },
  { type: "br", raw: "\n\n" },
  { type: "ext", raw: "xy", tokens: [{ type: "br", raw: "\n" }] },
];
console.log(count(ts));
for (const t of ts) console.log(kind(t));
