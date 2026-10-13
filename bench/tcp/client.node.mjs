// The client side on node:net (see client.bun.ts).
import net from "node:net"

const port = Number(process.env.PORT ?? 3310)
const conns = Number(process.env.CONNS ?? 64)
const size = Number(process.env.SIZE ?? 64)
const secs = Number(process.env.SECS ?? 5)
const msg = Buffer.alloc(size, 120)
let done = 0
let counting = false

for (let i = 0; i < conns; i++) {
  const s = net.connect({ host: "127.0.0.1", port, noDelay: true }, () => s.write(msg))
  let got = 0
  s.on("data", (d) => {
    got += d.length
    while (got >= size) {
      got -= size
      if (counting) done++
      s.write(msg)
    }
  })
}
setTimeout(() => {
  counting = true
  setTimeout(() => {
    console.log(JSON.stringify({ mps: done / secs }))
    process.exit(0)
  }, secs * 1000)
}, 1000)
