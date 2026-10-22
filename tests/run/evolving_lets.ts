class Heap {
  heap: number[] = [5, 3, 8, 1]
  bubbleUp(index: number = this.heap.length - 1): number {
    let parentIndex
    let steps = 0
    while (index > 0) {
      parentIndex = Math.floor((index - 1) / 2)
      steps++
      index = parentIndex
    }
    return steps
  }
}
function pick(n: number) {
  let label
  if (n > 2) label = "big"
  else label = "small"
  let found
  for (const x of [1, 2, 3]) if (x === n) found = x
  return `${label} ${found ?? "none"}`
}
console.log(new Heap().bubbleUp(), pick(3), pick(1), pick(7))
