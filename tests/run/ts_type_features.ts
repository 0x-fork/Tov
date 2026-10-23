// Template literal types (a string), `Uppercase`/`Capitalize` of literals, `satisfies`, and
// overload signatures choosing a call's result.

type Unit = "ms" | "s";
type Value = `${number}` | `${number}${Unit}` | `${number} ${Uppercase<Unit>}`;
type Loud = Uppercase<Unit>;
type Named = Capitalize<"ada" | "grace">;

function parse(v: Value): number;
function parse(v: number): string;
function parse(v: Value | number): number | string {
  if (typeof v === "number") return `${v}ms`;
  const n = parseFloat(v);
  return v.toLowerCase().endsWith("s") && !v.toLowerCase().endsWith("ms") ? n * 1000 : n;
}

const loud: Loud = "MS";
const who: Named = "Ada";
const config = { port: 8080, host: "localhost" } satisfies { port: number; host: string };
const doubled: number = parse("2s") * 2;
const text: string = parse(250).toUpperCase();
console.log(doubled, text, loud, who, config.port + 1, Number.isNaN(parse("x" as Value)));
