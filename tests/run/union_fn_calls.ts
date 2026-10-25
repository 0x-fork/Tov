type CbA = (r: number[]) => void;
type CbB = (r: number[] | undefined) => void;
let log: string[] = [];
function run(cb: CbA | CbB, give: number[]) { cb(give); }
const a: CbA = (r) => { log.push("a" + r.length); };
const b: CbB = (r) => { log.push("b" + (r ? r.length : -1)); };
run(a, [1, 2]); run(b, [1]);
type F1 = (x: number) => string;
type F2 = (x: number, y?: string) => string;
function call(f: F1 | F2): string { return f(3); }
console.log(log.join(","), call((x) => "f1:" + x), call((x: number, y?: string) => "f2:" + x + (y ?? "-")));
