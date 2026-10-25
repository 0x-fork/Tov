namespace Tokens {
  export interface Blockquote { type: "blockquote"; raw: string; text: string; tokens: Token[] }
  export interface Space { type: "space"; raw: string }
  export interface Generic { [index: string]: any; type: string; raw: string; tokens?: Token[] }
}
type Marked = Tokens.Blockquote | Tokens.Space;
type Token = Marked | Tokens.Generic;
function f(tokens: Token[]) {
  const lastToken = tokens.at(-1);
  if (lastToken?.type === "blockquote") {
    const old = lastToken as Tokens.Blockquote;
    old.text += "!";
    return old.text + old.tokens.length;
  }
  return "none";
}
const bq: Tokens.Blockquote = { type: "blockquote", raw: "> a", text: "a", tokens: [] };
console.log(f([bq]), f([{ type: "space", raw: " " }]), bq.text);
