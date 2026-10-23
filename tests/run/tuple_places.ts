// A tuple's element is a place: `edge[1] += w`, `t[0] = v`, `t[1]++`, seen by every reference to it.
const edges: [number, number][] = [[0, 5], [1, 7]];
for (const edge of edges) {
  edge[1] += 2;
  edge[0] = edge[0] * 10;
}
const t: [string, number] = ["a", 1];
t[1]++;
console.log(edges, t, t[0]);
const edges2: [number, number][] = [[0, 5], [1, 7]];
const e = edges2[0];
e[1] = 9;
const pairs = new Map<string, [string, number]>([["a", ["x", 1]]]);
const p = pairs.get("a")!;
p[1] += 41;
console.log(edges2, pairs);
