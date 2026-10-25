const tests: [RegExp, string][] = [
  [/^(\w+)\s+(\d+)/, "hello   42 world"],
  [/a+?b/, "xxaaab"],
  [/(?:ab|cd)+e/, "abcdabe"],
  [/^\s*$/, "   "],
  [/(\d{2,4})-(\d{1,2})/, "on 2024-7 ok"],
  [/(?=foo)fo/, "xfoo"],
  [/(?!foo)\w{3}/, "foobar"],
  [/\bcat\b/i, "the CAT sat"],
  [/(a)|(b)/, "b"],
  [/(\w)\1/, "abccd"],
  [/[^aeiou ]+/, "aeixyz"],
  [/x*/, "yyy"],
  [/^[\s\S]*?(?=\n|$)/, "line one\nline two"],
  [/(?:^|\n)(#{1,6}) (.*)/, "text\n## Head"],
  [/[a-c]{2}|z/i, "AbZ"],
  [/^ {0,3}(`{3,}|~{3,})([^`~\n]*)\n/, "```js\ncode"],
  [/.+@.+\..+/, "me@x.org"],
  [/\B\w/, "ab"],
  [/(a*)*b/, "aaab"],
  [/(?<y>\d{4})/, "in 1999"],
];
for (const [re, s] of tests) {
  const m = re.exec(s);
  console.log(String(re), m === null ? "null" : JSON.stringify([m.index, ...m]));
}
let s = "a1b22c333";
console.log(s.replace(/\d+/g, (d) => `<${d}>`), JSON.stringify(s.split(/\d+/)), "x-y_z".replace(/[-_]/g, " "));
const g = /o/g; let n = 0; while (g.exec("foo boo")) n++; console.log(n);
console.log(/^ab/m.test("x\nab"), "AbC".match(/b/i)?.index);
