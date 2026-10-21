// TypeScript's numbers computed as integers: past 2^53 they're what JavaScript's doubles hold
// (products and sums with products rounded, constant steps that stall at 2^53, literals), and
// they print as JavaScript prints them.
let seed = 12345
function rnd(n: number): number {
  seed = (seed * 1103515245 + 12345) % 2147483648
  return seed % n
}

function main() {
  const out: number[] = []
  for (let i = 0; i < 8; i++) out.push(rnd(1000))
  console.log(out.join(" "), seed)
  let h = 2166136261
  for (const ch of "hello world") h = ((h ^ ch.charCodeAt(0)) * 16777619) >>> 0
  console.log(h)
  const big = 9007199254740993
  console.log(big, big + 2, 2 ** 53 + 1, 9007199254740991 + 2, 123456789 * 987654321)
  let x = 4503599627370496
  for (let i = 0; i < 3; i++) x = x * 2 + 1
  console.log(x, x - 1, -x - 3, `${x}`, String(x * 3))
  let a = 9007199254740990
  for (let i = 0; i < 4; i++) a = a + 1
  let d = 9007199254740991
  d++
  d++
  console.log(a, d)
}

main()
