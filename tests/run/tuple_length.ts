type P = [number, string];
const p: P = [1, "a"];
const t3: [number, number, number] = [1, 2, 3];
console.log(p.length, t3.length, p.length + t3.length);
function f(x?: [number, number]): number | undefined { return x?.length; }
console.log(f([1, 2]), f());
