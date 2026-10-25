function first(src: string): string {
  let cap;
  if ((cap = /^a(\d)/.exec(src)) || (cap = /^b(\w)/.exec(src))) {
    return cap[0] + "/" + cap[1];
  }
  return "none";
}
console.log(first("a1x"), first("bzz"), first("c"));
function pick(a: string | undefined, b: string | undefined): string {
  let v: string | undefined;
  if ((v = a) || (v = b)) return v.toUpperCase();
  return "-";
}
console.log(pick("x", undefined), pick(undefined, "y"), pick(undefined, undefined));
function neg(x: number | undefined, y: number | undefined): number {
  if (!(x !== undefined && y !== undefined)) return -1;
  return x + y;
}
console.log(neg(1, 2), neg(undefined, 2));
