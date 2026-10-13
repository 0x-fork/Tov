// The client side: CONNS connections to an echo server on PORT, each sending a SIZE-byte message
// and waiting for its echo, again and again, for SECS seconds (after a second of warm-up).
// Prints the messages completed per second.
type State = { got: int }

const port = Number(process.env.PORT ?? 3310) ?? 3310
const conns = Number(process.env.CONNS ?? 64) ?? 64
const size = Math.trunc(Number(process.env.SIZE ?? 64) ?? 64)
const secs = Number(process.env.SECS ?? 5) ?? 5
const msg: u8[] = []
for (let i = 0; i < size; i++) {
  msg.push(120)
}
let done = 0
let counting = false

for (let i = 0; i < conns; i++) {
  try await Bun.connect<State>({
    hostname: "127.0.0.1",
    port,
    data: { got: 0 },
    socket: {
      open(socket) {
        socket.write(msg)
      },
      data(socket, data) {
        let got = socket.data!.got + data.length
        while (got >= size) {
          got -= size
          if (counting) done++
          socket.write(msg)
        }
        socket.data = { got }
      },
    },
  })
}
setTimeout(() => {
  counting = true
  setTimeout(() => {
    console.log(JSON.stringify({ mps: done / secs }))
    process.exit(0)
  }, secs * 1000)
}, 1000)
