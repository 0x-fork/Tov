// The Benchmarks Game mandelbrot (single-threaded): an N×N bitmap of the Mandelbrot set,
// 8 pixels to a byte. Instead of writing the PBM it prints the size and a checksum of the bytes.

function main() {
  const n = 4000
  const iter = 50
  const limit = 4.0
  let bytes = 0
  let checksum = 0
  for (let y = 0; y < n; y++) {
    const ci = (2.0 * y) / n - 1.0
    let byte = 0
    let bit = 0
    for (let x = 0; x < n; x++) {
      const cr = (2.0 * x) / n - 1.5
      let zr = 0.0
      let zi = 0.0
      let tr = 0.0
      let ti = 0.0
      for (let i = 0; i < iter && tr + ti <= limit; i++) {
        zi = 2.0 * zr * zi + ci
        zr = tr - ti + cr
        tr = zr * zr
        ti = zi * zi
      }
      byte <<= 1
      if (tr + ti <= limit) byte |= 1
      bit++
      if (bit === 8 || x === n - 1) {
        byte <<= 8 - bit
        checksum = (checksum * 31 + byte) & 0xffffff
        bytes++
        byte = 0
        bit = 0
      }
    }
  }
  console.log(`${n} ${bytes} ${checksum}`)
}

main()
