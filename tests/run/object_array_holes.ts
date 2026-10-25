interface Path { oldPos: number; last: string }
class Node2 { constructor(public v: number) {} }
const best: Path[] = new Array(5);
best[2] = { oldPos: 7, last: "x" };
const ns: Node2[] = new Array(3);
ns[1] = new Node2(4);
let found = 0;
for (let i = 0; i < 5; i++) { const p = best[i]; if (p) found += p.oldPos; }
for (let i = 0; i < 3; i++) { const n = ns[i]; if (n) found += n.v; else found += 100; }
console.log(found, best.length, ns.length);
