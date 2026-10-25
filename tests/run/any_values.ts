class Point {
  constructor(public x: number, public y: number) {}
  len(): number { return Math.sqrt(this.x * this.x + this.y * this.y); }
  scale(k: number): Point { return new Point(this.x * k, this.y * k); }
  get sum(): number { return this.x + this.y; }
}

// class through any
const p: any = new Point(3, 4);
console.log(p.x, p.y, p.len(), p.sum, p.scale(2).x);
p.x = 6;
console.log(p.x, p.len());
const back = p as Point;
console.log(back.y);

// JSON into any
const j: any = JSON.parse('{"a":[1,2,{"b":"c"}],"n":null,"t":true,"s":"str"}');
console.log(j.a.length, j.a[2].b, j.n, j.t, j.s.toUpperCase(), j.missing);
console.log(JSON.stringify(j));
j.extra = 5;
j.a.push(9);
console.log(j.a.length, j.extra + 1, JSON.stringify(j));
for (const x of j.a) console.log(typeof x);

// arrays of any
const xs: any[] = [1, "two", true, null, undefined, { k: 1 }];
for (const x of xs) console.log(typeof x, typeof x === "object" ? JSON.stringify(x) : x);
xs.push(42);
console.log(xs.length, xs[6] + 1);

// operators
const a: any = "5";
const b: any = 3;
console.log(a + b, b + a, a * b, b - 1, a > b, b < 10, a == 5, a === 5, -a, +a, ~b, b ** 2, b % 2);
let c: any = 1;
c += 2;
c++;
console.log(c);
const s: any = undefined;
console.log(s ?? "dflt", s || "or", s && "and", !s);
const o: any = {};
o.count = 0;
o.count++;
o.count += 10;
o["key"] = "v";
console.log(o.count, o.key, o["count"], JSON.stringify(o));

// functions as any
const fns: any = { add: (x: number, y: number) => x + y, greet: (n: string) => "hi " + n };
console.log(fns.add(2, 3), fns.greet("bob"));
const call = (f: any, args: any[]) => f(...args);
console.log(call((x: number) => x * 2, [21]));

// any to static
function total(xs: number[]): number { let t = 0; for (const x of xs) t += x; return t; }
const nums: any = [1, 2, 3.5];
console.log(total(nums));
const rec: any = { name: "n", age: 30 };
function show(r: { name: string; age: number }): string { return r.name + ":" + r.age; }
console.log(show(rec));

// string methods
const str: any = "  Hello, World  ";
console.log(str.trim().toLowerCase(), str.indexOf("o"), str.slice(2, 7), str.split(",").length, str.includes("World"), str.length);
