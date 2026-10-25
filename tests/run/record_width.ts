interface A { x?: number; y?: string }
interface B { z?: boolean }
type AB = A & B;
function use(o: AB): string { return `${o.x ?? 0}/${o.y ?? "-"}/${o.z ?? false}`; }
function f(opts: AB & { cb?: (n: number) => void }) {
  return use(opts);
}
function g(opts: ((n: number) => void) | (AB & { cb?: (n: number) => void }) = {}) {
  if (typeof opts === "function") return "fn";
  return use(opts);
}
console.log(f({ x: 1, cb: () => {} }), g({ y: "s" }), g(() => {}), g());
