// Counting into a map held by a local narrowed from `Map | undefined` (`if (!m) m = new Map()`).
const byUser = new Map<string, Map<string, number>>();
const recs = new Map<string, Record<string, number>>();
for (let i = 0; i < 40; i++) {
  const u = "u" + (i % 3), p = "/p/" + (i % 5);
  let m = byUser.get(u);
  if (!m) { m = new Map(); byUser.set(u, m); }
  m.set(p, (m.get(p) ?? 0) + 1);
  let r = recs.get(u);
  if (!r) { r = {}; recs.set(u, r); }
  r[p] = (r[p] ?? 0) + 2;
}
for (const [u, m] of byUser) console.log(u, [...m].map(([p, n]) => p + "=" + n).join(","), JSON.stringify(recs.get(u)));
