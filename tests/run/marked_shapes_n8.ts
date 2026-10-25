const a: any = "héllo";
const b: any = [1, "x", true];
const xs = [...a, ...b, 9];
console.log(xs.length, JSON.stringify(xs), [...a].length);
let d: any = 1;
d = "ab";
let e: any = 0;
e = [3, 4];
console.log(JSON.stringify([...d, ...e]));
