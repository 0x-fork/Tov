const mixed: (number | string | null)[] = [0, 1, "", "a", null];
console.log(mixed.filter(Boolean));
console.log(["1", "2.5", "x"].map(Number), [1, 2].map(String), ["3.5px"].map(parseFloat));
const xs: (number | string)[] = [0, "a", 2];
console.log(xs.filter((v) => Boolean(v)), Boolean(0), Boolean("0"), Boolean(NaN), Boolean());
