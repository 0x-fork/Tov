// The payments ledger on big.js (the most used decimal library for JavaScript): 2,000,000
// charges parsed from text, a 2.9% + 0.30 fee rounded to cents (half to even), net per merchant,
// payouts at 0.9234.
import Big from "big.js"

const N = 2000000
const MERCHANTS = 1000
const HALF_EVEN = 2

let seed = 42
function next(): number {
  seed = (seed * 48271) % 2147483647
  return seed
}

const rate = new Big("0.029")
const fixed = new Big("0.30")
const eur = new Big("0.9234")
const gross: Big[] = []
const net: Big[] = []
for (let i = 0; i < MERCHANTS; i++) {
  gross.push(new Big(0))
  net.push(new Big(0))
}
let fees = new Big(0)
for (let i = 0; i < N; i++) {
  const cents = Math.trunc(next() / 64) % 500000
  const text = `${Math.trunc(cents / 100)}.${`${cents % 100}`.padStart(2, "0")}`
  const amount = new Big(text)
  const fee = amount.times(rate).plus(fixed).round(2, HALF_EVEN)
  const m = Math.trunc(next() / 1024) % MERCHANTS
  gross[m] = gross[m].plus(amount)
  net[m] = net[m].plus(amount.minus(fee))
  fees = fees.plus(fee)
}
let totalGross = new Big(0)
let totalNet = new Big(0)
let payout = new Big(0)
for (let i = 0; i < MERCHANTS; i++) {
  totalGross = totalGross.plus(gross[i])
  totalNet = totalNet.plus(net[i])
  payout = payout.plus(net[i].times(eur).round(2, HALF_EVEN))
}
console.log(`gross ${totalGross.toFixed(2)}`)
console.log(`fees ${fees.toFixed(2)}`)
console.log(`net ${totalNet.toFixed(2)}`)
console.log(`payout EUR ${payout.toFixed(2)}`)
console.log(`merchant 7: ${gross[7].toFixed(2)} gross, ${net[7].toFixed(2)} net`)
