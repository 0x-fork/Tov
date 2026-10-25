const add = (a: number, b: number) => a + b;
console.log(add.call(null, 2, 3), add.apply(undefined, [4, 5]));
const bound = add.bind(null);
console.log(bound(6, 7));
const fns: ((s: string) => number)[] = [(s) => s.length];
console.log(fns[0].call({ lexer: 1 }, "abcd"));
const links: Record<string, { href: string }> = Object.create(null);
links["a"] = { href: "x" };
console.log(Object.keys(links).length, links.a.href);
