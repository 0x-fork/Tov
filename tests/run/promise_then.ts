// A promise's then, catch and finally, as in JavaScript: callbacks get the value or the error,
// what they return (or throw) settles the next promise, and a returned promise is waited for.
async function get(n: number): Promise<number> {
  await Promise.resolve(0)
  if (n < 0) throw new Error("negative")
  return n * 2
}

async function main() {
  await get(2).then((v) => console.log("then", v))
  await get(3).then((v) => get(v)).then((w) => console.log("chained", w))
  await get(-1).catch((e) => console.log("caught", e.message))
  await get(-1).then((v) => console.log("never", v), (e) => console.log("rejected", e.message))
  await get(1).finally(() => console.log("finally")).then((v) => console.log("after finally", v))
  await get(5).then((v) => {
    if (v > 5) throw new Error("too big " + v)
    return v
  }).catch((e) => console.log("then threw", e.message))
  const s = await get(4).then((v) => `s${v}`)
  console.log(s.toUpperCase())
  const n = await get(-2).catch(() => -1)
  console.log("fallback", n)
}

main()
