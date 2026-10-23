// An array of empty arrays, `[[]]`, takes its elements' type from where it's used, as `[]` does.

function edges(graph: number[][]): number {
  return graph.reduce((s, row) => s + row.length, 0);
}
const single = [[]];
const two = [[], []];
two[1].push(5);
console.log(edges(single), edges(two), two, [...Array(3).keys()]);
