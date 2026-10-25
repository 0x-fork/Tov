// `"key" in x`: narrowing unions of shapes, optional fields, classes, maps and unknown values.
type Circle = { kind: "c"; radius: number };
type Rect = { kind: "r"; w: number; h: number };
type Shape = Circle | Rect;
const area = (s: Shape): number => ("radius" in s ? Math.PI * s.radius * s.radius : s.w * s.h);
console.log(area({ kind: "c", radius: 1 }).toFixed(3), area({ kind: "r", w: 2, h: 3 }));
interface Opt { a: number; b?: string }
const os: Opt[] = [{ a: 1 }, { a: 2, b: "x" }];
console.log(os.map(o => ("b" in o) + "/" + ("a" in o) + "/" + ("c" in o)).join(" "));
class Animal { name = "a"; speak() { return "..."; } }
class Dog extends Animal { bark() { return "woof"; } }
const pets: Animal[] = [new Animal(), new Dog()];
console.log(pets.map(p => ("bark" in p) + "," + ("speak" in p) + "," + ("name" in p)).join(" "));
const rec: Record<string, number> = { x: 1 };
const m = new Map<string, number>([["y", 2]]);
console.log("x" in rec, "z" in rec, "y" in m);
function isObjWith(v: unknown, k: string): boolean { return typeof v === "object" && v !== null && k in v; }
console.log(isObjWith({ params: [1] }, "params"), isObjWith({ other: 1 }, "params"), isObjWith(5, "params"), isObjWith(new Dog(), "bark"));
