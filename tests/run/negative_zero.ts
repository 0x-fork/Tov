// Negative zero, as in TypeScript: the -0 literal, Object.is, and console.log writing -0.
const x: number = -0
const y = -0
const n = NaN
console.log(x, y, 1 / x, Object.is(y, -0), Object.is(y, 0), Object.is(n, NaN), Object.is("a", "a"), [-0, 0], -0 + 1, String(-0), `${-0}`)
console.log(-0, 0.5 - 0.5, -1 * 0.0, { z: -0 })
