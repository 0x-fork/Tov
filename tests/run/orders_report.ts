interface Order { id: number; customer: string; items: { sku: string; qty: number; price: number }[]; status: "open" | "shipped" | "cancelled" }
const customers = ["ann", "bob", "cat", "dan"];
const orders: Order[] = [];
for (let i = 0; i < 2000; i++) {
  const items = Array.from({ length: (i % 4) + 1 }, (_, j) => ({ sku: `S${(i + j) % 17}`, qty: (j % 3) + 1, price: ((i * 13 + j * 7) % 50) + 0.99 }));
  orders.push({ id: i, customer: customers[i % customers.length], items, status: i % 7 === 0 ? "cancelled" : i % 3 === 0 ? "shipped" : "open" });
}
const totals: Record<string, number> = {};
for (const o of orders) {
  if (o.status === "cancelled") continue;
  const t = o.items.reduce((s, it) => s + it.qty * it.price, 0);
  totals[o.customer] = (totals[o.customer] ?? 0) + t;
}
for (const [c, t] of Object.entries(totals).sort((a, b) => b[1] - a[1])) console.log(c, t.toFixed(2));
const bySku = new Map<string, number>();
for (const o of orders) for (const it of o.items) bySku.set(it.sku, (bySku.get(it.sku) ?? 0) + it.qty);
const top = [...bySku].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 3);
console.log(top);
const statusCounts = orders.reduce((acc, o) => { acc[o.status] = (acc[o.status] || 0) + 1; return acc; }, {} as Record<string, number>);
console.log(statusCounts, Object.keys(statusCounts).length);
const biggest = orders.filter((o) => o.status === "shipped").map((o) => ({ id: o.id, n: o.items.length })).sort((a, b) => b.n - a.n || a.id - b.id)[0];
console.log(biggest);
