function fix(src: string, name: string | RegExp, val: string): string { return src.replace(name, val); }
console.log(fix("a-b-a", "a", "x"), fix("a-b-a", /a/g, "y"), "1,2;3".split(Math.random() > 2 ? "," : /[,;]/).length);
