// The "1 billion nested loop iterations" program: a[i] += j % u over 10,000 x 100,000.
// u and r come from the argument count at runtime, so they can't be constant-folded.

function main() {
  const u = 40 + (process.argv.length - 2)
  let state = u
  for (let k = 0; k < 3; k++) {
    state = (state * 16807) % 2147483647
  }
  const r = state % 10000
  const a: number[] = []
  for (let i = 0; i < 10_000; i++) {
    a.push(0)
  }
  for (let i = 0; i < 10_000; i++) {
    for (let j = 0; j < 100_000; j++) {
      a[i]! += j % u
    }
    a[i]! += r
  }
  console.log(a[r]!)
}

main()
