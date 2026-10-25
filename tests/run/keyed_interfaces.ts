interface Generic { [index: string]: any; type: string; raw: string }
interface Code { type: "code"; raw: string; text: string; lang?: string }
class Cls { type = "cls"; raw = "c"; extra = 5; }
function code(s: string): Code | undefined { return s.startsWith("`") ? { type: "code", raw: s, text: s.slice(1) } : undefined; }
let g: Generic | undefined;
if (g = code("`abc")) {
  console.log(g.text, g.lang, g.nothing);
  g.text += "!";
  g.newKey = 3;
  console.log(g.text, g.newKey);
}
const o: Generic = { type: "x", raw: "y", more: "z" };
console.log(o.more, o.type);
o.more += "?"; o.added = [1, 2];
console.log(o.more, JSON.stringify(o.added));
const c: Generic = new Cls();
console.log(c.extra, c.type);
c.extra = 9;
console.log(c.extra);
