import type { Sale } from "./types.js";

export function parseCsv(text: string): Sale[] {
  const [header, ...lines] = text.trim().split("\n");
  const cols = header.split(",");
  return lines.map((line) => {
    const v = line.split(",");
    const row: Record<string, string> = {};
    cols.forEach((c, i) => (row[c] = v[i]));
    return { region: row.region, product: row.product, units: Number(row.units), price: parseFloat(row.price), date: row.date };
  });
}
