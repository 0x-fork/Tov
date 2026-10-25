const m = /(a)(x)?(b)?/.exec("za")!;
const href = m[2];
console.log(m[1], m[2] ? "x" : "-", m[3] || "none", href ? "has" : "none", m.length);
