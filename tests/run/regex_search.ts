// Searches with a first-character filter (the engine runs only where a match can start) and
// anchored ones, against what JavaScript gives: matches, groups, replacements, tests.
const pats: [string, string][] = [
  ["`([^`]+)`", "g"], ["\\*\\*([^*]+)\\*\\*", "g"], ["\\*([^*]+)\\*", "g"], ["\\[([^\\]]+)\\]\\(([^)]+)\\)", "g"],
  ["^(#{1,6})\\s+(.*)$", ""], ["^\\s*[-*]\\s+(.*)$", ""], ["^ab", "gm"], ["^", "gm"], ["^x|y", "g"], ["(a|b)c", "g"],
  ["(?:ab|cd)+e", "g"], ["(?<w>\\w+)@(\\w+)", "g"], ["[aeiou]{2}", "gi"], ["Hello", "gi"], ["\\d+", "g"], ["\\d*x", "g"],
  ["a?b", "g"], ["(a)|(b)", "g"], ["\\bfoo", "g"], ["(?=a)a", "g"], ["x{0,2}y", "g"], ["[^a-z]+", "g"], ["\\.", "g"],
  ["(\\d{3})-(\\d{4})", "g"], ["((a))", "g"], ["[\\]a]+", "g"], ["\\s\\S", "g"], ["$", "g"], ["q(?:u|v)", "gi"], ["^\\d", "m"],
];
const inputs = ["", "abc", "ab cd abcd e cde abcde", "**bold** and *em* and `code` [link](url)", "# Title\n## Sub\n- item\n * x", "Hello hello HELLO",
  "a1b22c333", "foo bar food xfoo", "555-1234 and 666-9876", "AEIOU aeiou xy", "user@host x@y", "q quv QU qv", "\n\nab\nab\r\nab", "aab bab cab", "xxy xy y"];
const out: string[] = [];
for (const [p, f] of pats) {
  for (const s of inputs) {
    const re = new RegExp(p, f);
    const ms: string[] = [];
    if (f.includes("g")) {
      for (const m of s.matchAll(re)) ms.push(`${m.index}:${m[0]}:${m.slice(1).join(",")}`);
    } else {
      const m = re.exec(s);
      if (m) ms.push(`${m.index}:${m[0]}:${m.slice(1).join(",")}`);
    }
    out.push(`${p}/${f} ${JSON.stringify(s)} => ${ms.join(" | ")} || ${s.replace(new RegExp(p, f), "<$&>")} || ${new RegExp(p, f.replace("g", "")).test(s)}`);
  }
}
console.log(out.join("\n"));
