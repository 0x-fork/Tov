const bs = (array: number[], target: number, start = 0, end = array.length - 1): number | null => {
  if (start > end) return null
  const mid = Math.floor((start + end) / 2)
  if (array[mid] === target) return mid
  return array[mid] < target ? bs(array, target, mid + 1, end) : bs(array, target, start, mid - 1)
}
function f(n: number, half = Math.floor(n / 2), big = n > 10, sq = n * n) { return [half, sq, big] }
console.log(bs([1, 3, 5, 7], 5), bs([1, 3], 4), f(7), f(11, 1))
