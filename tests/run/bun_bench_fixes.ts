// Behaviours Bun's benchmark snippets exercise: typed-array sort by number, array strings with
// holes, an async function returning a promise, a record captured and reassigned by a closure,
// spreads, Array.of, map, string search, lastIndexOf, JSON strings of every length.
const a = Int32Array.from([10, 9, 1, 100, -5, 0]);
a.sort();
console.log(a);
const f = new Float64Array([3.5, -0, 0, NaN, -Infinity, 2, -1.5]);
f.sort();
console.log(Array.from(f).join(" "), Object.is(f[2], -0), Object.is(f[3], 0));
console.log(new Uint8Array([200, 3, 50, 255, 0]).sort(), Float32Array.of(1.5, -2.25, 0.5).sort(), new Int16Array([300, -300, 7]).sort());
console.log([10, 9, 1, 100].sort());

const holes: (number | null | undefined)[] = [1, null, undefined, 2];
console.log(String(holes), `${holes}`, holes.toString(), holes.join("-"));

const one = async () => {
  return Promise.resolve(1);
};
async function two(x: number) {
  if (x > 0) return Promise.resolve(x * 2);
  return 0;
}
async function main() {
  console.log(await one(), await two(2), await two(-1));
}
main();

let foo = { a: 1 };
const bump = () => {
  foo = { a: foo.a + 1 };
};
bump();
bump();
console.log(foo);

const set = new Set(["x", "y", "z"]);
const nums = new Set([3, 1, 2]);
const copy = [...set];
copy.push("w");
console.log(copy, [...set], [...nums], [...[1, 2, 3]], Array.of(1, 2, 3), Array.of("a"));

const sq = (x: number) => x * x;
const xs = [1, 2, 3, 4];
console.log(xs.map(sq), xs.map((x, i) => x + i), ([] as number[]).map(sq), xs.map(String));

const urls = ["https://a.com/x", "http://b.org", "ftp://é.fr/ü"];
console.log(urls.map((u) => u.startsWith("https://")), urls.map((u) => u.endsWith("x")), urls.map((u) => u.substring(3, 9)), urls.map((u) => u.substring(9, 3)), "héllo".substring(1, 3), "abc".substring(-2, 99));

console.log([1, 2, 3, 2, 1].lastIndexOf(2), Int32Array.of(5, 6, 5).lastIndexOf(5), ["a", "b", "a"].lastIndexOf("a"), [1.5, NaN].lastIndexOf(NaN), new Uint8Array([1, 2, 3]).indexOf(3));

for (const n of [0, 1, 7, 15, 16, 17, 31, 32, 33, 64]) {
  const body = "a".repeat(n);
  const quoted = JSON.parse(`"${body}\\"${body}"`) as string;
  const escaped = JSON.parse(`"${body}\\n${body}\\u00e9"`) as string;
  console.log(n, quoted.length, escaped.length, (JSON.parse(`"${"🥟".repeat(n)}"`) as string).length);
}
try {
  console.log(JSON.parse(`"${"a".repeat(20)}\u0001"`) as string);
} catch (e) {
  console.log("control character rejected");
}

const words = /[a-z]+/g;
const sentence = "The quick brown fox jumps";
console.log(sentence.match(words), sentence.match(/[a-z]+/g), "x".match(words), "aaa".match(/a*/g), "a1b22".match(/\d+/g), "😀a😀bb".match(/[a-z]+/g));
const many = "ab ".repeat(100);
console.log(many.match(/b/g)!.length, words.lastIndex);
const fa = new Float64Array([3, -1, 2.5]);
console.log(fa, fa.slice().sort(), Float64Array.from([9, 1]).sort());
const re = /[a-z]+/g;
console.log("The quick fox".match(re), "abc".match(/(a)(b)?/), "The quick brown fox".match(/quick (\w+)/));
console.log(/(?<y>\d{4})-(?<m>\d\d)/.exec("on 2024-05-01 we went to the very long sentence here"));
console.log("zz".match(/(q)?z/));
