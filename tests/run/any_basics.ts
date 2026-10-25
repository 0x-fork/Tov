// options bags
interface Opts { ignoreCase?: boolean; n?: number }
function diff(a: string, b: string, options?: any): string {
  const ic = options?.ignoreCase ? "ic" : "cs";
  const n = options && options.n !== undefined ? options.n : 1;
  return a + b + ic + n;
}
console.log(diff("a", "b"));
console.log(diff("a", "b", { ignoreCase: true }));
console.log(diff("a", "b", { n: 3 }));
const o: Opts = { n: 5 };
console.log(diff("x", "y", o as any));

// any[] equality
function arrayEqual(a: any[], b: any[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}
console.log(arrayEqual([1, 2], [1, 2]), arrayEqual(["a"], ["b"]));

// any locals
const merged: any = { a: 1, b: "two" };
merged.c = [1, 2, 3];
console.log(merged.a + 1, merged.b.length, merged.c.length, typeof merged.d);
let v: any = 5;
v = v * 2;
console.log(v, typeof v);
v = "str";
console.log(v.toUpperCase(), v.length);


// functions in any
const cb: any = (x: number) => x * 3;
console.log(cb(4));
const replacer = (k: string, v: any): any => (typeof v === "number" ? v * 10 : v);
console.log(replacer("k", 2), replacer("k", "s"));
