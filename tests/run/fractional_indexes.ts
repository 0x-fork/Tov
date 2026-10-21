// Fractional indexes, as JavaScript reads them: slice, substring, charAt and at truncate them.
const r = "abcdef"
const half = r.length / 2
console.log(r.slice(0, half), r.slice(half), [1,2,3,4].slice(0, 4 / 3), "abcd".substring(1.7, 3.2), "hello".charAt(1.9), [5,6,7].at(1.5), "xyz".at(-1.2))
