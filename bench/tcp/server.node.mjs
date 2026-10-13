// The echo server on node:net (node:tls with TLS=1).
import net from "node:net"
import tls from "node:tls"
import { readFileSync } from "node:fs"

const port = Number(process.env.PORT ?? 3300)
const dir = process.env.CERT_DIR ?? "."
const echo = (s) => s.on("data", (d) => s.write(d))
const server = process.env.TLS === "1"
  ? tls.createServer({ cert: readFileSync(`${dir}/cert.pem`), key: readFileSync(`${dir}/key.pem`) }, echo)
  : net.createServer({ noDelay: true }, echo)
server.listen(port, "127.0.0.1")
