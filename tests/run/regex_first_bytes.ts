// Where a match can start, found from the pattern (`a?b`, ` {0,3}#`, lookaheads, `\b`, anchored
// alternatives): the same matches as without.
const cases: [RegExp, string][] = [
  [/^ {0,3}#{1,6} /, "   ## title"],
  [/^ {0,3}#{1,6} /, "    ## code"],
  [/^(?:\*+|_+)x/, "**x"],
  [/^<!--|^<\/[a-z]+>|^<[a-z]+>/, "</div> rest"],
  [/^<!--|^<\/[a-z]+>|^<[a-z]+>/, "text <b>"],
  [/a?b/, "xxab"],
  [/(?=\w)\d+/, "ab 12"],
  [/\bfoo/, "xfoo foo"],
  [/c{0,2}d/, "ccd"],
  [/(?:ab)*c/, "ababc"],
  [/x*?y/, "zzxxy"],
  [/^ *([^\n ].*)\n/, "  head\nnext"],
  [/^(?!\s)[a-z]+/, " no"],
  [/^(?!\s)[a-z]+/, "yes"],
];
for (const [re, s] of cases) {
  const m = re.exec(s);
  console.log(re.source, JSON.stringify(s), m === null ? null : [m.index, m[0]]);
}
