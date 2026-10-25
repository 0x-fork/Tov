// A recursive alias inside a union with functions, reached through a class's map (the cycle
// collector's visitors), a class holding a record that points back to it, typeof and type guards
// on members of the alias, narrowing back to it, and `as` casting its arrays.
type Val = number | string | Val[] | Fn | null;
interface Fn { params: string[]; body: Val; env: Env }
class Env { vars = new Map<string, Val | ((args: Val[]) => Val)>(); f: Fn | null = null; }
function isFn(v: unknown): v is Fn { return typeof v === "object" && v !== null && !Array.isArray(v); }
const env = new Env();
env.vars.set("+", (a: Val[]) => (a[0] as number) + (a[1] as number));
env.vars.set("id", { params: ["x"], body: "x", env });
env.vars.set("n", 5);
env.vars.set("s", "str");
env.vars.set("l", ["a", "b"]);
env.vars.set("z", null);
for (const k of ["+", "id", "n", "s", "l", "z", "missing"]) {
  const f = env.vars.get(k);
  const kind = typeof f === "function" ? "call " + f([2, 3]) : isFn(f) ? "fn " + f.params.join(",") : Array.isArray(f) ? "list " + (f as string[]).join("|") : "value " + String(f);
  console.log(k, typeof f, kind);
}
env.f = { params: [], body: 1, env };
console.log(env.f.env === env, [1, "x", [2, ["y"]], null].join(" "));
// (`unknown` parameters given members of the alias kept with `undefined` as a niche)
type Tree = number | Tree[];
const trees = new Map<string, Tree>([["leaf", 1], ["branch", [1, [2, 3]]]]);
const isArr = (v: unknown): boolean => Array.isArray(v);
const kindOf = (v: unknown): string => typeof v;
for (const k of ["leaf", "branch", "none"]) console.log(k, isArr(trees.get(k)), kindOf(trees.get(k)), Array.isArray(trees.get(k)));
