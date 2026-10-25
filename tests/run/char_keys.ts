// One-character keys beside longer ones, control and non-ASCII characters, read by for...of.
const text = "héllo wörld\tab\u0001c 😀a😀 the end";
const counts = new Map<string, number>();
for (const ch of text) counts.set(ch, (counts.get(ch) ?? 0) + 1);
counts.set("ab", 10);
counts.set("a", (counts.get("a") ?? 0) + 100);
console.log([...counts].map(([k, n]) => JSON.stringify(k) + "=" + n).join(" "));
console.log(counts.get("a"), counts.get("ab"), counts.get("😀"), counts.get("\t"), counts.has("b"), counts.has("z"));
const seen = new Set<string>();
for (const ch of "mississippi") seen.add(ch);
for (let i = 0; i < 128; i++) seen.add(String.fromCharCode(i));
console.log(seen.size, [...seen].slice(0, 4).join("|"));
const chars: string[] = [];
for (const ch of "aé中😀z") chars.push(ch);
console.log(chars.length, chars.join(","));
