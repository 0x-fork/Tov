// Namespaces: their declarations used inside, as `N.x` in the module and from modules that import N
// (types, functions, constants, classes; a namespace merged with a function of its name).
namespace Shapes {
  export interface Circle { kind: "circle"; r: number }
  export interface Square { kind: "square"; s: number }
  export type Shape = Circle | Square;
  export const PI2 = Math.PI * 2;
  export function area(x: Shape): number { return x.kind === "circle" ? x.r * x.r * Math.PI : x.s * x.s; }
  export class Box { constructor(public items: Shape[]) {} total(): number { return this.items.reduce((a, b) => a + area(b), 0); } }
  function hidden(): string { return "h"; }
  export const tag = () => hidden() + PI2.toFixed(2);
}
const c: Shapes.Circle = { kind: "circle", r: 2 };
const b = new Shapes.Box([c, { kind: "square", s: 3 }]);
console.log(Shapes.area(c).toFixed(3), b.total().toFixed(3), Shapes.tag(), Shapes.PI2 > 6);
import { Tokens, type Token } from "./namespaces/tokens.ts";
import { assertType } from "./namespaces/at.ts";
const toks: Token[] = [{ type: "space", raw: " " }, Tokens.make(" hi ")];
function show(t: Token): string { return t.type === "text" ? t.text : "_"; }
const sp: Tokens.Space = { type: "space", raw: "  " };
console.log(toks.map(show).join(","), sp.raw.length, Tokens.VERSION);
const f: assertType.Flag = true;
assertType<number>(1);
console.log(f, assertType.id);
