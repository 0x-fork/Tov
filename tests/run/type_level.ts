// Type-level operators: mapped types (over literals, keyof, of class methods), conditional types
// (distributing, with infer), constant keys in type literals, this parameters, narrowing by an
// assignment in a condition (a let a closure also assigns too).
type Width = "narrow" | "wide";
type M<T> = { [K in Width]: T };
const m: M<number> = { narrow: 1, wide: 2 };
type R<T> = T extends string ? number : boolean;
const x: R<string> = 3;
const y: R<number> = true;
type Opt<T> = { [K in keyof T]?: T[K] };
interface P { a: number; b: string }
const o: Opt<P> = { a: 5 };
type Req<T> = { [K in keyof T]-?: T[K] };
const q: Req<Opt<P>> = { a: 1, b: "z" };
type ElemOf<A> = A extends (infer E)[] ? E : never;
const e: ElemOf<string[]> = "elem";
type NonStr<T> = T extends string ? never : T;
const ns: NonStr<string | number | boolean> = 7;
const KING = "k";
const QUEEN = "q";
function rights(): { [KING]: boolean; [QUEEN]: boolean } { return { k: true, q: false }; }
type Fn = (this: P, n: number) => number;
const f: Fn = (n) => n * 2;
console.log(m.narrow + m.wide, x, y, o.a, o.b, q.b, e, ns, rights().k, f(4));
class H2 { a(x: string): string { return x; } b(n: number): number { return n; } }
type HO2 = { [K in keyof H2]?: (...args: Parameters<H2[K]>) => ReturnType<H2[K]> };
const ho: HO2 = { a: (x: string) => x + '!' };
console.log(ho.a ? ho.a('hi') : 'none');
interface Tok2 { raw: string }
function next2(s: string): Tok2 | undefined { return s ? { raw: s[0] } : undefined; }
function run2(src: string): number {
  let n = 0;
  let token: Tok2 | undefined;
  [1].some(() => { if (token = next2(src)) { return true; } return false; });
  if (token = next2(src)) {
    n += token.raw.length;
  }
  let t2: Tok2 | undefined;
  if (t2 = next2(src)) n += t2.raw.length;
  return n;
}
console.log(run2("abc"));
