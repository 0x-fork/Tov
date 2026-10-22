import type { Summary } from "./lib/index.js";
import { levelOf } from "./lib/index.js";

export default function table(rows: Summary[]): string {
  const lines = rows.map((r) => `${r.region.padEnd(8)} ${r.revenue.toFixed(2).padStart(14)} ${String(r.units).padStart(8)}  ${r.top.padEnd(10)} ${levelOf(r.revenue)}`);
  return ["region         revenue    units  top        level", ...lines].join("\n");
}
