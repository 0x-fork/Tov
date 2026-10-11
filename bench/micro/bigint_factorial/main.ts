// 40,000! by multiplying up from 1 (bigint * bigint where one side is small), then its 166,714
// decimal digits: the digit count, the digit sum and the trailing zeros.
const N = 40000

let f = 1n
for (let i = 2; i <= N; i++) {
  f *= BigInt(i)
}
const s = f.toString()
let sum = 0
let zeros = 0
for (let i = 0; i < s.length; i++) {
  sum += s.charCodeAt(i) - 48
}
for (let i = s.length - 1; i >= 0 && s[i] === "0"; i--) {
  zeros++
}
console.log(`${N}! has ${s.length} digits`)
console.log(`digit sum ${sum}, trailing zeros ${zeros}`)
