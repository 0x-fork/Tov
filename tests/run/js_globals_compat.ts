// concat of values and arrays, the URI functions, a reassigned promise, Promise.all/race of
// plain values, Object.hasOwn and hasOwnProperty.
const a = [1, 2, 3];
console.log(a.concat(4, 5), a.concat([6], 7, [8, 9]), a.concat(), a.concat([10]), ["x"].concat("y", ["z"]));
const enc = encodeURIComponent("a b&c=d/é😀?");
console.log(enc, decodeURIComponent(enc), encodeURI("http://x.com/a b?q=1&r=é#h"), decodeURI("http://x.com/a%20b%3Fc"), decodeURIComponent("a%40"));
try { decodeURIComponent("%E0%A4%A"); } catch (e) { console.log((e as Error).name, (e as Error).message); }
try { decodeURIComponent("%zz"); } catch (e) { console.log((e as Error).name); }
async function main() {
  let p = Promise.resolve();
  p = p.then(() => {});
  await p;
  console.log(await Promise.all([1, 2]), await Promise.race([7, 8]));
}
main();
const o = { a: 1, b: undefined as number | undefined };
const d: Record<string, number> = { x: 1 };
console.log(Object.hasOwn(o, "a"), Object.hasOwn(d, "x"), Object.hasOwn(d, "y"), o.hasOwnProperty("a"), d.hasOwnProperty("q"));
const ao = { async async(n: number) { return n + 1; } };
const p = { async(x: number) { return x * 2; }, delete(y: number) { return y; } };
async function main2() { console.log(await ao.async(1), p.async(3), p.delete(4)); }
const q = { async: 5 };
console.log(q.async);
main2();
