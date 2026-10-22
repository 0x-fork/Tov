const heap = [9, 4, 7]
let i = 0
let steps = 0
while (heap[2 * i + 1] || heap[2 * i + 2]) {
  steps++
  i = 2 * i + 1
}
const seen: boolean[] = [true]
console.log(steps, i, seen[0] && seen[5] ? "both" : "not both", !heap[10])
