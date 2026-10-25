const sum = (...xs: number[]) => xs.reduce((a, b) => a + b, 0);
console.log(sum(), sum(1), sum(1, 2, 3));
const tag = (name: string, ...parts: string[]): string => name + ":" + parts.join(",");
console.log(tag("a"), tag("b", "x", "y"));
const nums = [4, 5, 6];
console.log(sum(...nums), sum(0, ...nums, 10));
function apply(f: (...args: number[]) => number, xs: number[]): number { return f(...xs); }
console.log(apply(sum, [7, 8]));
type Logger = (...msgs: string[]) => void;
const log: Logger = (...msgs) => console.log(msgs.length, msgs.join(" "));
log("hello", "world");
log();
const fns: Array<(...a: number[]) => number> = [sum, (...a) => a.length];
for (const f of fns) console.log(f(1, 2, 3));
class K { run(...xs: number[]) { return xs.length; } }
console.log(new K().run(1, 2));
const sum2 = (...xs: number[]) => xs.reduce((a, b) => a + b, 0);
function both(f: (a: number, b: number) => number): number { return f(3, 4); }
console.log(both(sum2));
function many(f: (...xs: number[]) => number): number { return f(1, 2, 3, 4); }
console.log(many((a: number, b: number) => a * 10 + b));
console.log(many((a: number, ...more: number[]) => a + more.length * 100));
function joinAll(sep: string, ...parts: string[]): string { return parts.join(sep); }
const j: (sep: string, ...p: string[]) => string = joinAll;
console.log(j("-", "a", "b", "c"));
const pairs: [string, (...a: number[]) => number][] = [["sum2", sum2], ["max", (...a) => Math.max(...a)]];
for (const [n, f] of pairs) console.log(n, f(5, 9, 2));
const xs = [1, 2, 3].map((x) => sum2(x, x));
console.log(xs.join(","));
