import { Sale, Summary, Level } from "./types.js";

export const levelOf = (revenue: number): Level => (revenue > 1_000_000 ? Level.High : revenue > 100_000 ? Level.Mid : Level.Low);

export function summarize(sales: Sale[]): Summary[] {
  const byRegion = new Map<string, { revenue: number; units: number; products: Map<string, number> }>();
  for (const s of sales) {
    let r = byRegion.get(s.region);
    if (!r) {
      r = { revenue: 0, units: 0, products: new Map() };
      byRegion.set(s.region, r);
    }
    r.revenue += s.units * s.price;
    r.units += s.units;
    r.products.set(s.product, (r.products.get(s.product) ?? 0) + s.units);
  }
  return [...byRegion.entries()]
    .map(([region, r]) => ({ region, revenue: Math.round(r.revenue * 100) / 100, units: r.units, top: [...r.products.entries()].sort((a, b) => b[1] - a[1])[0][0] }))
    .sort((a, b) => b.revenue - a.revenue);
}
