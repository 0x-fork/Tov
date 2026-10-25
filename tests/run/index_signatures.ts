interface Generic { [index: string]: any; type: string; raw: string; tokens?: string[] }
const g: Generic = { type: "ext", raw: "xy", extra: 42, name: "nm" };
console.log(g.type, g.raw, g.extra, g.name.toUpperCase(), g.missing, g.tokens);
interface Opts { verbose: boolean; [key: string]: number | boolean }
const o: Opts = { verbose: true, depth: 3 };
console.log(o.verbose, o.depth, o.width);
interface Para { type: "paragraph"; raw: string; text: string }
interface Br { type: "br"; raw: string }
type Tok = Para | Br | Generic;
const toks: Tok[] = [{ type: "paragraph", raw: "p", text: "hello" }, { type: "br", raw: "\n" }, { type: "custom", raw: "c", text: "ctext" }];
for (const t of toks) {
  if (t.type === "paragraph" || t.type === "custom") console.log(t.type, t.text);
}
interface Dict { [k: string]: number }
const d: Dict = { a: 1, b: 2 };
d.c = 3;
console.log(d.a + d.b + d.c, Object.keys(d).length);
