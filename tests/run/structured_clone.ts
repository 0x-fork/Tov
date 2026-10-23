// `structuredClone(v)`: a deep copy of data, changing it leaves the original as it was.

const graph: number[][] = [[0, 4, Infinity], [4, 0, 1], [Infinity, 1, 0]];
const dist = structuredClone(graph);
dist[0][2] = 5;
dist.push([9]);
console.log(graph, dist);

type Node = { id: string; edges: { to: string; w: number }[]; seen?: boolean };
const nodes: Node[] = [{ id: "a", edges: [{ to: "b", w: 1 }] }, { id: "b", edges: [] }];
const copy = structuredClone(nodes);
copy[0].edges[0].w = 7;
copy[1].seen = true;
console.log(JSON.stringify(nodes), JSON.stringify(copy));

const m = new Map<string, number[]>([["x", [1, 2]]]);
const mc = structuredClone(m);
mc.get("x")!.push(3);
mc.set("y", []);
const s = structuredClone(new Set(["p", "q"]));
s.add("r");
console.log(m, mc, s, structuredClone("text"), structuredClone(3.5), structuredClone(undefined));

const maybe: (number[] | null)[] = [[1], null];
const mcopy = structuredClone(maybe);
mcopy[0]!.push(2);
console.log(maybe, mcopy);
