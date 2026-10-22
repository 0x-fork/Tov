// A small TypeScript project as tsc (NodeNext) and Bun take it: `.js` specifiers for `.ts`
// files, a barrel with `export *` and `export type`, a default export, renamed imports.
import path from "node:path";
import { parseCsv as parse, summarize } from "./ts_project/lib/index.js";
import table from "./ts_project/format.js";
import * as lib from "./ts_project/lib/index";
import type { Sale } from "./ts_project/lib/types.ts";

const regions = ["north", "south", "east", "west", "central"];
const products = ["widget", "gadget", "doohickey", "gizmo", "thing"];
let seed = 7;
const rnd = () => (seed = (seed * 16807) % 2147483647);
const rows = ["region,product,units,price,date"];
for (let i = 0; i < 2000; i++) rows.push(`${regions[rnd() % 5]},${products[rnd() % 5]},${1 + (rnd() % 20)},${(rnd() % 10000) / 100},2024-0${1 + (rnd() % 9)}-1${rnd() % 10}`);
const sales: Sale[] = parse(rows.join("\n"));
console.log(table(summarize(sales)));
console.log(`${sales.length} rows from ${path.basename("/tmp/data.csv")}`, lib.levelOf(2_000_000));
