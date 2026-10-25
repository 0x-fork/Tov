console.log(JSON.stringify(["a,b,c,d".split(",", 2), "a,b".split(",", 0), "a,b".split(",", 5), "x y z".split(" ", undefined)]));
const lim: number | undefined = Math.random() > 2 ? 1 : undefined;
console.log("q-r-s".split("-", lim).length);
const m = /(\w+) (\w+)/.exec("hello world")!;
m[2] = m[2].toUpperCase();
m[0] = "replaced";
console.log(m[0], m[1], m[2]);
