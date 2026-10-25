function show(cap: string[]): string { return cap.length + ":" + cap.join("|"); }
const m = /(a)(x)?(b)/.exec("zab")!;
console.log(show(m), m[1]);
