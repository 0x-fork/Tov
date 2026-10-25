interface Link { href: string; title: string | null }
function render({ href, title }: Link): string {
  href = href.toUpperCase();
  if (title === null) title = "none";
  return href + "|" + title;
}
console.log(render({ href: "a", title: null }), render({ href: "b", title: "t" }));
const q: { src: string; n: number }[] = [{ src: "a", n: 1 }, { src: "b", n: 2 }];
q.at(-1)!.src = "changed";
q.at(0)!.n += 10;
console.log(JSON.stringify(q));
class Lexer {
  state: { inLink: boolean; depth: number };
  queue: string[];
  helper: Helper;
  constructor() {
    this.helper = new Helper();
    this.helper.owner = this;
    this.queue = [];
    this.state = { inLink: false, depth: 0 };
  }
}
class Helper { owner?: Lexer; }
const lx = new Lexer();
console.log(lx.helper.owner === lx, lx.state.depth, lx.queue.length);
