// Sequences of characters, classes and greedy quantifiers (matched without backtracking) next to
// ones that need the engine: same matches, groups and replacements either way.
const pats: RegExp[] = [/\{\{(\w+)\}\}/g, /(\d+)-(\d+)/g, /a{1,3}b/g, /a{1,3}c?/g, /\s*,\s*/g, /^(\w+)=(.*)$/, /[a-z]+@[a-z]+\.com/g, /x*/g, /(a)(b)?c/g, /((a)b)c/g, /(?:ab)c(d)/g, /\d{2,}/g, /[^,]+/g, /a.c/g, /A+b/gi, /(\w+)\s(\w+)/, /\$(\d+)\.(\d\d)/g, /a+a/g, /\w+\d/g, /[\d.]+/g, /\.{3}/g, /-?\d+(?:\.\d+)?/g, /^$/g, /()/g, /c$/g, /^ab/g, /[A-Z][a-z]*/g, /\W+/g, /a{2}/g, /\S+@\S+/g];
const inputs = ["{{name}} and {{ x }} {{a1}}{{b}}", "12-34 5-6 7-", "aaaab aab b ab", "aaaa aac", " a , b ,c,, d ", "key=value", "foo@bar.com x@y.com", "xxx", "abc ac abbc", "abcd abc", "1 22 333 4.5.6", "a,b,,c", "abc a-c a\nc", "AAb aab Ab", "hello world again", "$12.50 $3.99 $1.5", "aaa a", "abc1 x2 3", "...... ..", "-7 3.14 -0.5 8.", "", "abc c", "ab abab", "Hello World ABC", "a--b  c", "aaaaa"];
for (const re of pats) {
  const lines: string[] = [];
  for (const s of inputs) {
    re.lastIndex = 0;
    const ms: string[] = [];
    if (re.global) for (const m of s.matchAll(re)) ms.push(m.index + ":" + [...m].map(x => x === undefined ? "~" : x).join("|"));
    else { const m = re.exec(s); if (m) ms.push(m.index + ":" + [...m].map(x => x === undefined ? "~" : x).join("|")); }
    lines.push(ms.join(";") + " # " + s.replace(re, "<$&>") + " # " + s.replace(re, (m: string) => "[" + m.length + "]"));
  }
  console.log(String(re), JSON.stringify(lines));
}
