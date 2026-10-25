interface T1 { type: "text"; text: string }
interface T2 { type: "esc"; text: string }
interface T3 { type: "tag"; text: string; raw: string }
class R { text(t: T1 | T2): string { return "<" + t.text + ">"; } br(token: { n: number }): string { return "<br" + token.n + ">"; } }
class P { text(t: T1 | T2 | T3): string { return t.text; } br(): string { return "\n"; } }
function run(r: R | P, t: T1 | T2): string { return r.text(t) + r.br({ n: 2 }); }
console.log(run(new R(), { type: "text", text: "a" }), JSON.stringify(run(new P(), { type: "esc", text: "b" })));
