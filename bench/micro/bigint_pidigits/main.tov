// Benchmarks Game pidigits: the first N digits of pi from a spigot over growing bigints.
const N = 10000

let acc = 0n
let den = 1n
let num = 1n
let k = 0
let i = 0
let line = ""
let out = ""
while (i < N) {
  k++
  const k2 = BigInt(k * 2 + 1)
  acc = (acc + num * 2n) * k2
  den = den * k2
  num = num * BigInt(k)
  if (num > acc) {
    continue
  }
  const d3 = (num * 3n + acc) / den
  const d4 = (num * 4n + acc) / den
  if (d3 !== d4) {
    continue
  }
  line += d3
  i++
  if (i % 10 === 0) {
    out += line + "\t:" + i + "\n"
    line = ""
  }
  acc = (acc - den * d3) * 10n
  num = num * 10n
}
process.stdout.write(out)
