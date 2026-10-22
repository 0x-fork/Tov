class N { children = new Map<string, N>(); v = 1; kids: N[] = []; f(): number { return this.v; } }
const root = new N();
root.children.set("a", new N());
root.kids.push(new N());
let h: N | undefined = root;
console.log(h?.children.get("a")?.v, h?.children.size, h?.kids[0].v, h?.f(), h?.children.get("zz")?.v);
const pick = (n: number): N | undefined => n > 0 ? root : undefined; h = pick(0);
console.log(h?.children.get("a"), h?.children.size, h?.kids[0].v, h?.f());
const o: { a?: { b: { c: number[] } } } = { a: { b: { c: [7] } } };
console.log(o.a?.b.c[0], o.a?.b.c.length);
const o2: { a?: { b: { c: number[] } } } = {};
console.log(o2.a?.b.c[0], o2.a?.b.c.length);
const fs: Array<(() => () => number) | undefined> = [() => () => 3, undefined];
console.log(fs[0]?.()(), fs[1]?.()());
