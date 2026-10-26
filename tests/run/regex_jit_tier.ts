// Expressions built at run time and used enough times to be compiled by the JIT tier, against
// what JavaScript gives: loops' groups, empty iterations, backreferences, anchors, classes.
const cases: [string, string, string[]][] = [
  ["\\b\\w[^]??|", "i", ["\nb1x__Ab", " c", ""]],
  ["a[]|b[^]c", "", ["abxc", "b\nc", "a"]],
  ["(?:(a)|b)+", "", ["ab", "ba", "bab"]],
  ["(a*)+", "", ["aa", "b", ""]],
  ["(?:|a)?b", "", ["ab", "b"]],
  ["(?:a*?)*x", "", ["aax", "x"]],
  ["(?:(a)\\1)+", "", ["aaaa", "aab"]],
  ["\\[(?:[^\\[\\]`]|(?<a>`+)[^`]+\\k<a>(?!`))*?\\]\\((?:\\\\[\\s\\S]|[^\\\\()]|\\((?:\\\\[\\s\\S]|[^\\\\()])*\\))*\\)", "g", ["see [a `b` c](http://x.y/(z)) and [d](e)", "[`x`](y)"]],
  ["(?<=ab)c|(?<!x)d", "", ["abc", "xd", "d"]],
  ["^b|c$", "m", ["a\r\nb", "a\rb", "c\r\nd", "xc\n"]],
  ["a.b", "", ["a\rb", "a\nb", "a b"]],
  ["a.b", "s", ["a\rb", "a\nb"]],
  ["[a-z]+\\d", "i", ["XyZ9", "a1"]],
  ["\\u{61}+", "u", ["baaa"]],
  ["\\x41|\\u0042|\\cJ", "", ["zA", "B", "x\ny"]],
  ["foo", "y", ["foofoo", "xfoo"]],
  ["(\\w)(\\d)?", "g", ["a1b c3"]],
  ["(\\b)( {1,3}1\\s[a-c]|(^c{0,2}?[ab]?)*?|b{0,}(\\d+c[^]*[^a]){1,}\\S{1,}?)(?<n8>\\4{1,}|(\\x61{1,3}(?:){0,2}?(?:\\w?a[\\w-]?\\b|\\x61{1,}[^\\n])[\\w-])$|a*((\\5{0,2}x{1,}?){1,}\\d*\\B|1{0,}\\s*?(?:_b{0,}\\3?))*c{0,2}\\3+){1,}1", "", ["aaa_1BaabbA", " 1bc bbc1aAB"]],
  ["^(?:[ \\t]*(?:\\n|$))+", "", ["  \n\t\n x", "\n", ""]],
  ["(?:\\n(?!(a)|b)c)*", "", ["\nc\nc\na", "\nb"]],
  ["x{2,3}?y|z{0,0}w", "", ["xxxy", "w"]],
  ["(?:a|ab)(?:c|bcd)(d*)", "", ["abcd"]],
  ["\\p{Lu}+\\P{L}", "u", ["abCD1", "XY "]],
  ["[\\p{P}\\p{S}]+", "u", ["a*_~b"]],
];
for (const [src, flags, inputs] of cases) {
  const re = new RegExp(src, flags);
  let line = "";
  for (let pass = 0; pass < 4; pass++) {
    line = "";
    for (const s of inputs) {
      re.lastIndex = 0;
      for (let k = 0; k < 4; k++) {
        const m = re.exec(s);
        if (!m) { line += " null"; break; }
        line += " " + m.index + ":" + JSON.stringify(m.slice(0));
        if (!re.global && !re.sticky) break;
        if (m[0].length === 0) re.lastIndex++;
      }
      line += " test=" + re.test(s);
      line += " |";
    }
  }
  console.log(JSON.stringify(src) + "/" + flags + line);
}
