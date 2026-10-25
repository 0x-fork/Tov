interface P { type: "p"; raw: string; text: string; tokens: string[] }
interface S { type: "s"; raw: string }
interface T { type: "t"; raw: string; text: string; tokens?: string[] }
type Tok = P | S | T;
const items: { tokens: Tok[] }[] = [{ tokens: [{ type: "p", raw: "r", text: "x", tokens: ["a"] }] }, { tokens: [{ type: "s", raw: "q" }] }];
for (const item of items) {
  if (item.tokens[0] && "tokens" in item.tokens[0] && item.tokens[0].tokens) {
    item.tokens[0].text = "[" + item.tokens[0].text + "]";
    item.tokens[0].tokens.unshift("first");
  }
}
console.log(JSON.stringify(items));
