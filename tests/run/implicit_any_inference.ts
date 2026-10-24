// What TypeScript leaves `any` (fields with no type, defaults, objects given as dictionaries,
// generic classes made with no type arguments), typed by what the program does with them.

var last = 42, A = 3877, C = 29573, M = 139968;
function rand(max: number) {
    last = (last * A + C) % M;
    return max * last / M;
}

const CURVE = { P: 2n ** 61n - 1n, n: 1000003n };
function mod(a: bigint, b = CURVE.P) {
    const r = a % b;
    return r >= 0n ? r : b + r;
}

class P {
    static ZERO = new P(0n, 1n);
    constructor(public x: bigint, public y: bigint) {}
    add(o: P): P { return new P(this.x + o.x, this.y + o.y); }
}

class LRU {
    private size
    private m
    constructor(size: number) {
        this.size = size;
        this.m = new Map()
    }
    get(key: bigint) {
        if (this.m.has(key)) {
            const v = this.m.get(key)
            this.m.delete(key)
            this.m.set(key, v)
            return v
        }
        return null
    }
    put(key: bigint, value: bigint) {
        if (this.m.has(key)) {
            this.m.delete(key)
        } else if (this.m.size >= this.size) {
            this.m.delete(this.m.keys().next().value)
        }
        this.m.set(key, value)
    }
}

class Box<T> {
    items: T[] = [];
    put(x: T) { this.items.push(x); }
    first(): T { return this.items[0]!; }
}

var IUB = { a: 0.27, c: 0.12, g: 0.12, t: 0.27 };
function makeCumulative(table: { [key: string]: number }) {
    var prev = null;
    for (var c in table) {
        if (prev) table[c] += table[prev];
        prev = c;
    }
}

function main() {
    console.log(rand(1).toFixed(6), rand(10).toFixed(6), last);
    console.log(mod(-5n), mod(17n, 5n));
    console.log(P.ZERO.add(new P(2n, 3n)).y);
    const lru = new LRU(2);
    lru.put(1n, 10n); lru.put(2n, 20n); lru.get(1n); lru.put(3n, 30n);
    console.log(lru.get(2n), lru.get(1n), lru.get(3n));
    const b = new Box();
    b.put("a"); b.put("b");
    console.log(b.first(), b.items.length);
    makeCumulative(IUB);
    console.log(IUB.t.toFixed(2));
    console.log(0 < CURVE.n, 1e30 < 10n ** 40n, 5n < 5.5, 5n > NaN, 5n <= Infinity);
    console.log(Deno.args.length);
}
main();
