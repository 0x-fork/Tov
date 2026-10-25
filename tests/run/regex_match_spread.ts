const m = /(\d+)-(\d+)?-(x)/.exec("12--x")!;
console.log(JSON.stringify([...m]), [...m].length);
for (const g of m) console.log(g === undefined ? "undef" : g);
const all = [..."a1b22c333".matchAll(/\d+/g)].map((x) => [...x][0]);
console.log(all.join(","));
