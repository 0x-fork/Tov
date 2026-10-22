export const countingSort = (input: number[], min: number, max: number) => {
  const sorted = []
  const count = new Array(max - min + 1).fill(0)
  for (const x of input) count[x - min]++
  for (let i = 1; i < count.length; i++) count[i] += count[i - 1]
  for (let i = input.length - 1; i >= 0; i--) {
    sorted[--count[input[i] - min]] = input[i]
  }
  return sorted
}
