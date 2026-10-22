const init_graph = (N: number): [number, number][][] => {
  const graph = Array(N)
  for (let i = 0; i < N; ++i) {
    graph[i] = []
  }
  return graph
}
const transpose = (graph: number[][]): number[][] => {
  const t = Array(graph.length)
  for (let i = 0; i < graph.length; ++i) t[i] = []
  for (let i = 0; i < graph.length; ++i) for (const j of graph[i]) t[j].push(i)
  return t
}
const g = init_graph(3)
g[0].push([1, 5])
console.log(g, transpose([[1], [2], [0]]))
const sq = new Array(4)
for (let i = 0; i < 4; i++) sq[i] = i * i
console.log(sq)
