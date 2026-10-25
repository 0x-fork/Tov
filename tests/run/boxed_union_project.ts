namespace Tokens {
  export interface Blockquote { type: "blockquote"; raw: string; text: string; tokens: Token[] }
  export interface Space { type: "space"; raw: string }
  export interface Generic { [index: string]: any; type: string; raw: string; tokens?: Token[] }
}
type Marked = Tokens.Blockquote | Tokens.Space;
type Token = Marked | Tokens.Generic;
const bq: Tokens.Blockquote = { type: "blockquote", raw: "> a", text: "a", tokens: [] };
const ts: Token[] = [bq];
const t = ts[0];
if (t.type === "blockquote") { const o = t as Tokens.Blockquote; console.log(o.text); }
