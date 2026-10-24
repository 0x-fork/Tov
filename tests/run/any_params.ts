// A parameter's `any` is a type parameter of its own, bound by each call; `: any` as a result is inferred.
function linearSearch(array: any[], target: any): number {
  for (let i = 0; i < array.length; i++) if (array[i] === target) return i;
  return -1;
}
function first(xs: any[]): any { return xs[0]; }
console.log(linearSearch([1, 2, 3], 2), linearSearch(["a", "b"], "c"), first([5, 6]) + 1, first(["x"]).length);
