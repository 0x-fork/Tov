// An object literal indexed by any string is a dictionary; `as const` strings keep their literal types.
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c);
console.log(esc(`<a href="x">Tom & 'Jerry'</a>`));
const tiers = ["free", "pro", "team"] as const;
type Tier = typeof tiers[number];
function price(t: Tier): number { return t === "free" ? 0 : t === "pro" ? 10 : 25; }
let total = 0;
for (let i = 0; i < 6; i++) total += price(tiers[i % 3]);
console.log(total, tiers.join("/"));
const unit = (k: string) => ({ s: 1, m: 60, h: 3600 })[k];
console.log(unit("m"), unit("h"));
