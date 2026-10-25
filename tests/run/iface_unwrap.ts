interface Generic { [index: string]: any; type: string; raw: string }
interface Text { type: "text"; raw: string; text: string }
interface Space { type: "space"; raw: string }
type Tok = Text | Space | Generic;
function text(s: string): Text | undefined { return s.length > 1 ? { type: "text", raw: s, text: s.toUpperCase() } : undefined; }
function space(s: string): Space | undefined { return s === " " ? { type: "space", raw: s } : undefined; }
const out: Tok[] = [];
for (const s of ["ab", " ", "cd", "x"]) {
  let t: Generic | undefined;
  if (t = text(s)) { out.push(t); continue; }
  if (t = space(s)) { out.push(t); continue; }
}
for (const t of out) {
  if (t.type === "text") { t.text += "!"; console.log("text", t.raw, t.text); }
  else console.log(t.type, JSON.stringify(t.raw));
}
