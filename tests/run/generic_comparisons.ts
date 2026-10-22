function shellSort<T>(arr: T[]): T[] {
  for (let gap = arr.length >> 1; gap > 0; gap >>= 1) {
    for (let i = gap; i < arr.length; i++) {
      const temp = arr[i]
      let j = i
      while (j >= gap && arr[j - gap] > temp) {
        arr[j] = arr[j - gap]
        j -= gap
      }
      arr[j] = temp
    }
  }
  return arr
}
function max<T>(a: T, b: T): T { return a >= b ? a : b }
console.log(shellSort([5, 3, 8, 1]), shellSort(["pear", "apple", "fig"]), shellSort([0.5, -2.5, 1]))
console.log(max(true, false), max([1, 2], [1, 10]), max("b", "a"), max(3, 7))
