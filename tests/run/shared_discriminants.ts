// Two variants sharing a discriminant: a literal is the one its fields fit, and switch, if and ?: on it see both.
interface Space { type: "space"; raw: string }
interface HTML { type: "html"; raw: string; block: boolean; text: string }
interface Tag { type: "html"; raw: string; text: string }
type MT = Space | HTML | Tag;
function t1(t: MT): string { if (t.type === "html") return "H:" + t.text; return "S"; }
function t2(t: MT): string { return t.type !== "html" ? "notH" : t.text; }
function t3(t: MT): string { switch (t.type) { case "html": return t.text + "!"; default: return t.raw; } }
const xs: MT[] = [{ type: "html", raw: "r", text: "tag" }, { type: "html", raw: "r2", block: true, text: "blk" }, { type: "space", raw: " " }];
for (const x of xs) console.log(t1(x), t2(x), t3(x), x.type === "html");
interface Space2 { type: 'space'; raw: string }
interface HTML2 { type: 'html'; raw: string; block: boolean; text: string }
interface Tag2 { type: 'html'; raw: string; text: string }
type MT2 = Space2 | HTML2 | Tag2;
function html2(t: HTML2 | Tag2): string { return t.text; }
function go2(token: MT2): string {
  switch (token.type) {
    case 'space': return 'S';
    case 'html': return html2(token);
  }
}
const h: Tag2 = { type: 'html', raw: '<a>', text: '<a>' };
console.log(go2(h));
