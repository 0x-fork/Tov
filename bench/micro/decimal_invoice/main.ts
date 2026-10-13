// Invoices on big.js: 200,000 invoices of 1 to 9 lines (price × quantity), each with a lump-sum
// discount allocated across its lines in proportion to their totals (a division per line,
// rounded to cents half to even), then 8.25% tax per line, rounded to cents.
import Big from "big.js"

const INVOICES = 200000
const HALF_EVEN = 2
Big.DP = 34

let seed = 7
function next(): number {
  seed = (seed * 48271) % 2147483647
  return seed
}

const rate = new Big("0.0825")
let grand = new Big(0)
let taxes = new Big(0)
let discounts = new Big(0)
for (let inv = 0; inv < INVOICES; inv++) {
  const lines = 1 + (Math.trunc(next() / 1024) % 9)
  const totals: Big[] = []
  let sum = new Big(0)
  for (let l = 0; l < lines; l++) {
    const price = new Big(Math.trunc(next() / 1024) % 100000).div(100)
    const qty = 1 + (Math.trunc(next() / 1024) % 12)
    const total = price.times(qty)
    totals.push(total)
    sum = sum.plus(total)
  }
  if (sum.eq(0)) {
    continue
  }
  const discount = new Big(Math.trunc(next() / 1024) % 5000).div(100)
  for (let l = 0; l < lines; l++) {
    const share = discount.times(totals[l]).div(sum).round(2, HALF_EVEN)
    const net = totals[l].minus(share)
    const tax = net.times(rate).round(2, HALF_EVEN)
    grand = grand.plus(net).plus(tax)
    taxes = taxes.plus(tax)
    discounts = discounts.plus(share)
  }
}
console.log(`grand total ${grand.toFixed(2)}`)
console.log(`tax ${taxes.toFixed(2)}`)
console.log(`discounts ${discounts.toFixed(2)}`)
