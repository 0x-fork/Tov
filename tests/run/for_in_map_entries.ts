// `for (k in o)` over a dictionary: its own entries, `o[k]` the current one's value, unless the
// body may change which keys it holds.
function cumulative(table: { [key: string]: number }) {
    let last: string | null = null;
    for (const c in table) {
        if (last) table[c] += table[last];
        last = c;
    }
}

function pick(table: { [key: string]: number }, r: number): string {
    for (const c in table) {
        if (r < table[c]) return c;
    }
    return "?";
}

function main() {
    const t: { [key: string]: number } = { a: 0.25, b: 0.25, c: 0.5 };
    cumulative(t);
    console.log(t, pick(t, 0.3), pick(t, 0.9), pick(t, 2));
    // (deleting as it goes: keys not yet visited aren't, as in JavaScript)
    const d: { [key: string]: number } = { x: 1, y: 2, z: 3 };
    for (const k in d) {
        console.log(k, d[k]);
        delete d["z"];
    }
    // (adding as it goes)
    const g: { [key: string]: string } = { p: "1" };
    let n = 0;
    for (const k in g) {
        if (n++ < 3) g[k + "x"] = g[k] + "!";
    }
    console.log(Object.keys(g).join(","), n);
    let sum = 0;
    for (const k of Object.keys(t)) {
        t[k] *= 2;
        sum += t[k];
    }
    console.log(sum);
}
main();
