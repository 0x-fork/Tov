interface Generic { [index: string]: any; type: string; raw: string }
const g: Generic = { type: "ext", raw: "xy", extra: 42 };
g.extra = 43;
g.added = "new";
g.raw = "r2";
console.log(g.extra, g.added, g.raw, g["extra"], g["type"]);
console.log(JSON.stringify(g));
console.log(Object.keys(g).join(","));
const k = "added";
console.log(g[k]);
interface Para { type: "paragraph"; raw: string; text: string }
interface Text { type: "text"; raw: string; text: string }
type Tok = Para | Text | Generic;
const ts: Tok[] = [{ type: "paragraph", raw: "p", text: "a" }, { type: "text", raw: "t", text: "b" }];
for (const t of ts) {
  t.raw += "!";
  if (t.type === "paragraph" || t.type === "text") t.text += "?";
}
console.log(JSON.stringify(ts));
interface G3 { [k: string]: any; type: string }
const g3: G3 = { type: "x", a: 1 };
console.log(Object.keys(g3).join(","), "a" in g3, "type" in g3, "zz" in g3);
const h3: G3 = { ...g3, b: 2 };
console.log(JSON.stringify(h3));
const p3: G3 = JSON.parse('{"type":"y","q":[1,2]}');
console.log(p3.type, p3.q.length, JSON.stringify(p3));
