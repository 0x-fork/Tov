const cache: RegExp[] = [];
function get(i: number): RegExp {
  let r = cache[i];
  if (!r) {
    r = new RegExp("a{" + (i + 1) + "}");
    cache[i] = r;
  }
  return r;
}
console.log(get(2).source, get(0).source, get(2).source, cache.length);
const ns: number[] = [];
ns[3] = 7;
console.log(ns.length, ns[3]);
