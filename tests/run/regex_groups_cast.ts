// `if (!m?.groups) return` narrows `m` and its groups; `m.groups as { ... }` reads them as a
// record (a group that didn't take part is `undefined`).

function parse(str: string): number {
  const match = /^(?<value>-?\d*\.?\d+) *(?<unit>ms|s)?$/i.exec(str);
  if (!match?.groups) return NaN;
  const { value, unit = "ms" } = match.groups as { value: string; unit: string | undefined };
  return parseFloat(value) * (unit.toLowerCase() === "s" ? 1000 : 1);
}
console.log(parse("100"), parse("2s"), parse("1.5 S"), parse("x"));
