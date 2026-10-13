// An echo server: every byte read is written back. PORT; TLS=1 serves TLS with cert.pem and
// key.pem from CERT_DIR.
import { readFileSync } from "node:fs"

const port = Number(process.env.PORT ?? 3300)
const dir = process.env.CERT_DIR ?? "."
const tls = process.env.TLS === "1" ? { cert: readFileSync(`${dir}/cert.pem`, "utf8"), key: readFileSync(`${dir}/key.pem`, "utf8") } : undefined

Bun.listen({
  hostname: "127.0.0.1",
  port,
  tls,
  socket: {
    data(socket, data) {
      socket.write(data)
    },
  },
})
