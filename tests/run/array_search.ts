// indexOf / lastIndexOf / includes on each kind of element: ints, numbers (NaN, -0), strings, objects, bytes.
const ints = [5, 3, 9, 3, -1];
const nums = [1.5, NaN, -0, 2, NaN];
const strs = ["a", "bb", "", "bb", "é"];
const o = { k: 1 }, objs = [{ k: 1 }, o, { k: 1 }];
const bytes = new Uint8Array([1, 255, 7, 255]);
const flags = [false, true, false];
console.log(ints.indexOf(3), ints.lastIndexOf(3), ints.includes(-1), ints.indexOf(4));
console.log(nums.indexOf(NaN), nums.includes(NaN), nums.indexOf(0), nums.includes(-0), nums.lastIndexOf(2));
console.log(strs.indexOf("bb"), strs.lastIndexOf("bb"), strs.includes(""), strs.indexOf("é"), strs.includes("b" + "b"), strs.indexOf("x"));
console.log(objs.indexOf(o), objs.includes({ k: 1 }), bytes.indexOf(255), bytes.lastIndexOf(255), bytes.includes(8), flags.indexOf(true), flags.lastIndexOf(false));
console.log(([] as number[]).indexOf(1), ([] as string[]).includes("a"));
