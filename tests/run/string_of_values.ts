// String(x) of numbers and unions of strings, numbers, booleans and nothing; the same numbers made
// text again and again (cached); record lookups by keys that are and aren't the same string.
const vals: (string | number | boolean | undefined | null)[] = ["a", 1.5, 2, -0, true, false, undefined, null, NaN, 1e21, 0.1 + 0.2];
console.log(vals.map(v => String(v)).join("|"));
let n = 0;
const seen = new Set<string>();
for (let r = 0; r < 3; r++) for (let i = 0; i < 2000; i++) { const s = String(i / 8); seen.add(s); n += s.length; }
console.log(n, seen.size, String(1 / 3), String(-1 / 3), String(123.456));
const row: Record<string, string | number> = { id: 7, city: "Oslo", spend: 12.5 };
const key = ["sp", "end"].join("");
console.log(row["spend"], row[key], row["city"], row["c" + "ity"], row["none"] ?? "missing");
