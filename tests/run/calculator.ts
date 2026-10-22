type Tok = { kind: "num"; value: number } | { kind: "op"; op: string } | { kind: "paren"; open: boolean };
function lex(src: string): Tok[] {
  const out: Tok[] = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === " ") { i++; continue; }
    if (/[0-9.]/.test(c)) {
      let j = i;
      while (j < src.length && /[0-9.]/.test(src[j])) j++;
      out.push({ kind: "num", value: parseFloat(src.slice(i, j)) });
      i = j;
    } else if ("+-*/^".includes(c)) { out.push({ kind: "op", op: c }); i++; }
    else if (c === "(" || c === ")") { out.push({ kind: "paren", open: c === "(" }); i++; }
    else throw new Error(`bad char ${c}`);
  }
  return out;
}
function parse(toks: Tok[]): number {
  let pos = 0;
  const peek = () => toks[pos];
  function primary(): number {
    const t = toks[pos++];
    if (t.kind === "num") return t.value;
    if (t.kind === "op" && t.op === "-") return -primary();
    if (t.kind === "paren" && t.open) { const v = expr(); pos++; return v; }
    throw new Error("unexpected");
  }
  function power(): number { const b = primary(); const t = peek(); if (t && t.kind === "op" && t.op === "^") { pos++; return b ** power(); } return b; }
  function term(): number { let v = power(); for (let t = peek(); t && t.kind === "op" && (t.op === "*" || t.op === "/"); t = peek()) { pos++; v = t.op === "*" ? v * power() : v / power(); } return v; }
  function expr(): number { let v = term(); for (let t = peek(); t && t.kind === "op" && (t.op === "+" || t.op === "-"); t = peek()) { pos++; v = t.op === "+" ? v + term() : v - term(); } return v; }
  return expr();
}
for (const e of ["1 + 2 * 3", "(1 + 2) * 3", "2 ^ 3 ^ 2", "-4 + 10 / 4", "3.5 * (2 - 0.5)"]) console.log(e, "=", parse(lex(e)));
let sum = 0;
for (let i = 0; i < 20000; i++) sum += parse(lex(`${i} * 2 + (${i % 7} - 3) / 2`));
console.log(sum);
try { lex("2 $ 3"); } catch (e) { console.log((e as Error).message); }
