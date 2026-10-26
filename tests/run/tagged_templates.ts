const name = "world";
const n = 42;
console.log(String.raw`a\nb${name}c\t${n}`);
console.log(String.raw`C:\Users\${name}\file`);

function tag(strings: TemplateStringsArray, ...values: unknown[]): string {
  let out = "";
  for (let i = 0; i < strings.length; i++) {
    out += "[" + strings[i] + "]";
    if (i < values.length) out += "<" + String(values[i]) + ">";
  }
  return out + " raw=" + strings.raw.join("|");
}
console.log(tag`hello ${name}, you are ${n}!\n`);
console.log(tag`plain`);

function sql(strings: TemplateStringsArray, ...params: (string | number)[]) {
  return { text: strings.reduce((acc, s, i) => acc + "$" + i + s), values: params };
}
const q = sql`SELECT * FROM t WHERE a = ${1} AND b = ${"x"}`;
console.log(q.text, JSON.stringify(q.values));

const html = (s: TemplateStringsArray, ...v: string[]) => s.map((x, i) => x + (v[i] ?? "")).join("");
console.log(html`<p>${"hi"}</p>`);
console.log(String.raw`\unicode and \xyz \u{zz}`);
console.log(String.raw`path\to\file`.length);
const parts: string[] = [];
const collect = (s: TemplateStringsArray, ...v: number[]) => { parts.push(s.join("_") + v.join("+")); return v.length; };
console.log(collect`a${1}b${2}c`, collect``, parts.join(" "));
