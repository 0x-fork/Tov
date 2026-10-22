class Tree { constructor(public left: Tree | null = null, public right: Tree | null = null) {} }
const root = new Tree(new Tree(new Tree()), new Tree());
function depth(t: Tree | null = root): number {
  return t ? 1 + Math.max(depth(t.left), depth(t.right)) : 0;
}
console.log(depth(), depth(null), depth(root.right));
function label(s: string | null = "none"): string { return s === null ? "null" : s; }
console.log(label(), label(null), label("x"));
const [a = 1, b = 2] = [null, undefined];
console.log(a, b);
const { p = "p", q = "q" }: { p?: string | null; q?: string | null } = { p: null };
console.log(p, q);
function opts({ w = 3, h = 4 }: { w?: number; h?: number | null } = {}): string { return `${w}x${h}`; }
console.log(opts(), opts({ h: null }), opts({ w: 7 }));
