// A `Record<string, V>` filled by key and read by name: fields, not a map.
const header = "region,product,units,price,date";
const lines = ["north,widget,3,1.5,2024-01-01", "south,gadget,7,2.25,2024-02-02", "east,gizmo,1,9.5,2024-03-03"];
const cols = header.split(",");
for (const line of lines) {
  const v = line.split(",");
  const row: Record<string, string> = {};
  cols.forEach((c, i) => (row[c] = v[i]));
  console.log(row.region, row["product"], Number(row.units) * parseFloat(row.price), row.date);
}
function parse(text: string): { a: string; b: number }[] {
  const out: { a: string; b: number }[] = [];
  for (const l of text.split("\n")) {
    const kv: Record<string, string> = {};
    for (const part of l.split(";")) {
      const [k, val] = part.split("=");
      kv[k] = val;
    }
    out.push({ a: kv.a ?? "none", b: kv.b !== undefined ? Number(kv.b) : -1 });
  }
  return out;
}
console.log(parse("a=x;b=2;c=9\nb=5\na=y;zz=1"));
const rec: Record<string, number> = {};
const keys = ["one", "two"];
for (const k of keys) rec[k] = k.length;
rec.three = 3;
console.log(rec.one + rec.two + rec.three);
const seen: Record<string, number> = {};
for (const w of ["a", "b", "a"]) seen[w] = (seen[w] ?? 0) + 1;
console.log(seen);
