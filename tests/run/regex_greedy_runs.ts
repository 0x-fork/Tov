// Greedy repeats of one character (\S+, [^:]*, .{2,}): one opcode takes the longest run and gives
// characters back one at a time; captures, lookarounds and Unicode around them.
// greedy repeats of one character: matches, captures around them, giving back
const cases: [RegExp, string][] = [
  [/^(\S+) \S+ \S+ \[([^:]+):(\d{2})/, "10.1.2.3 - - [24/Oct/2026:13:55:36 +0000]"],
  [/a+b/, "caaab"], [/a+ab/, "caaab"], [/a*aab/, "aaab"], [/x*/, "yyy"], [/(\w+)@(\w+)\.com/, "mail bob@site.com now"],
  [/.+?,/, "a,b,c"], [/.+,/, "a,b,c"], [/\d{2,4}/, "123456"], [/\d{2,4}5/, "123456"], [/[a-z]{3}/i, "xyABCd"],
  [/^.*$/m, "line one\nline two"], [/\s+/g, "a  b\t\tc"], [/[^"]+"/, 'say "hi" now'], [/(?:ab)+/, "ababab"],
  [/é+/u, "caféééx"], [/[à-ÿ]+/, "xàéîz"], [/😀+/u, "a😀😀b"], [/.{3,}/s, "ab\ncd"], [/(a+)+b/, "aaab"],
  [/\bw\w*/g, "word wide web"], [/[0-9a-f]+$/i, "id=DEADbeef"], [/(?=x)x+/, "axxx"], [/q+(?!r)/, "qqqr"],
  [/(?<=\$)\d+/, "cost $42"], [/a{0,2}a{2}/, "aaa"], [/[a-c]*c/, "abcabc"], [/\S*\d/, "abc1def2 x"],
];
for (const [re, s] of cases) {
  const m = re.exec(s);
  console.log(re.source, m ? JSON.stringify(Array.from({ length: m.length }, (_, k) => m[k] ?? null)) + " @" + m.index : "null", s.replace(re, "<$&>"), s.split(re).length);
}
let n = 0;
const line = "2026-10-24 12:00:01 GET /api/items/42 200 1234 Mozilla/5.0";
for (let i = 0; i < 50000; i++) { const m = /^(\S+) (\S+) (\w+) ([^ ]+) (\d+) (\d+)/.exec(line); if (m) n += m[5].length + m[4].length; }
console.log(n);
const more: [RegExp, string][] = [[/\d+5/, "12345"], [/[^,]+,x/, "ab,cd,x"], [/\w+ x/, "aa bb x"], [/(\S+)z/, "abz abzz"], [/a+a/, "aaa"], [/[a-z]+(\d)/, "abc1"], [/.+:(.+):/, "a:b:c:"], [/\s+$/, "x   "]];
for (const [re, s] of more) { const m = re.exec(s); console.log(re.source, m ? m[0] + "|" + (m[1] ?? "") + "@" + m.index : "null"); }
