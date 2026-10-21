const m = new Map<string, Map<string, string>>([["w", new Map([["c", "r"]])]]);
console.log(m.get("w")?.get("c"), m.get("w")?.get("x"), m.get("z")?.get("c"));
const db: Record<number, { name: string }> = { 1: { name: "Ann" } };
function find(id: number): string {
  const u = db[id];
  if (u) return u.name;
  return "missing";
}
console.log(find(1), find(2));
const xs = [10, 20];
const at = xs[5];
console.log(at ?? "none", at === undefined);
function delay(ms: number, v: string, fail = false): Promise<string> {
  return new Promise((res, rej) => setTimeout(() => (fail ? rej(new Error(v)) : res(v)), ms));
}
async function main() {
  const settled = await Promise.allSettled([delay(20, "a"), delay(5, "b", true), delay(1, "c")]);
  for (const s of settled) {
    console.log(s.status, s.status === "fulfilled" ? s.value : String(s.reason));
  }
}
main();
