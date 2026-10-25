interface Generic { [index: string]: any; type: string; raw: string; tokens?: Tok[] }
interface Heading { type: "heading"; raw: string; depth: number; text: string; tokens: Tok[] }
interface Space { type: "space"; raw: string }
type Tok = Heading | Space | Generic;
class T {
  heading(s: string): Heading | undefined { return s.startsWith("#") ? { type: "heading", raw: s, depth: 1, text: s.slice(1), tokens: [] } : undefined; }
  space(s: string): Space | undefined { return s === " " ? { type: "space", raw: s } : undefined; }
}
function lex(srcs: string[]): Tok[] {
  const t = new T();
  const out: Tok[] = [];
  for (const src of srcs) {
    let token: Generic | undefined;
    if (token = t.heading(src)) { out.push(token); continue; }
    if (token = t.space(src)) {
      const last = out.at(-1);
      if (last !== undefined) last.raw += "+"; else out.push(token);
      continue;
    }
  }
  return out;
}
const r = lex(["#a", " ", "#b", " "]);
console.log(r.length, r.map(x => x.type + ":" + x.raw).join(","));
