interface Links { [k: string]: { href: string } }
type Token = { type: string; raw: string };
type TokensList = Token[] & { links: Links };

class Lexer {
  tokens: TokensList;
  constructor() {
    this.tokens = [] as unknown as TokensList;
    this.tokens.links = Object.create(null);
  }
  lex(src: string): TokensList {
    for (const w of src.split(" ")) {
      if (w.startsWith("[")) this.tokens.links[w.slice(1, -1)] = { href: "/" + w };
      this.tokens.push({ type: "word", raw: w });
    }
    return this.tokens;
  }
}

const t = new Lexer().lex("a [b] c [d]");
console.log(t.length, Object.keys(t.links).length, t.links["b"].href);
for (const tok of t) console.log(tok.type, tok.raw);
const u = [] as unknown as TokensList;
u.links = {};
u.links.x = { href: "y" };
console.log(u.links.x.href, u.length);
