class Html { text(t: string): string { return "<p>" + t + "</p>"; } br(): string { return "<br>"; } }
class Plain { text(t: string): string { return t; } br(): string { return "\n"; } }
function render(r: Html | Plain, words: string[]): string {
  let out = "";
  for (const w of words) out += r.text(w) + r.br();
  return out;
}
console.log(render(new Html(), ["a", "b"]));
console.log(JSON.stringify(render(new Plain(), ["a", "b"])));
class Lexer<T = string> {
  static lex<U = string>(src: string): string[] { return src.split(" "); }
}
const lexFn = Lexer.lex;
console.log(lexFn("x y z").length);
