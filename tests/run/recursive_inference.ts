function fact(n: number) {
  if (n <= 1) return 1
  return n * fact(n - 1)
}
function qs(a: number[]) {
  if (a.length <= 1) return a
  const [p, ...rest] = a
  return [...qs(rest.filter(x => x < p)), p, ...qs(rest.filter(x => x >= p))]
}
function walk(n: number, out: string[]) {
  if (n == 0) return out
  out.push(`n${n}`)
  walk(n - 1, out)
  return out
}
function fib(n: number) {
  if (n < 2) return n
  const a = fib(n - 1)
  return a + fib(n - 2)
}
const QuickSort = (array: number[], left: number = 0, right: number = array.length - 1) => {
  if (left < right) {
    const p = array[right]
    let i = left
    for (let j = left; j < right; j++) if (array[j] < p) { [array[i], array[j]] = [array[j], array[i]]; i++ }
    ;[array[i], array[right]] = [array[right], array[i]]
    QuickSort(array, left, i - 1)
    QuickSort(array, i + 1, right)
  }
  return array
}
const depth = (s: string): number => s.length == 0 ? 0 : 1 + depth(s.slice(1))
function outer(n: number) {
  const go = (k: number) => {
    if (k == 0) return 0
    return k + go(k - 1)
  }
  return go(n)
}
function generate(n: number): string[] {
  const out: string[] = []
  const solve = (s: string, open: number, close: number) => {
    if (s.length === 2 * n) {
      out.push(s)
      return
    }
    if (open < n) solve(s + '(', open + 1, close)
    if (close < open) solve(s + ')', open, close + 1)
  }
  solve('', 0, 0)
  return out
}
function partition(array: number[], left: number, right: number) {
  const pivot = array[right]
  let i = left - 1
  let j = right
  while (i < j) {
    while (array[++i] < pivot);
    while (array[--j] > pivot);
    if (i < j) [array[i], array[j]] = [array[j], array[i]]
  }
  ;[array[right], array[i]] = [array[i], array[right]]
  return i
}
console.log(fact(10), qs([3, 1, 2, 5, 4]), walk(3, []), fib(20))
console.log(QuickSort([5, 2, 9, 1, 7]), depth("abc"), outer(4))
console.log(generate(3))
const xs = [3, 9, 1, 7, 0, 5]
console.log(partition(xs, 0, 5), xs)
const fs = [0.5, 1.5]
const at = [-1, 0, 1, 2]
console.log(fs[5] > 0, fs[-1] < 0, 2 < fs[1], fs[1] >= 1.5, at.map(i => xs[i] <= 3))
