// The Benchmarks Game mandelbrot (single-threaded): an N×N bitmap of the Mandelbrot set,
// 8 pixels to a byte. Instead of writing the PBM it prints the size and a checksum of the bytes.

fn main() {
    let n: i32 = 4000;
    let iter = 50;
    let limit = 4.0;
    let mut bytes: i64 = 0;
    let mut checksum: i64 = 0;
    for y in 0..n {
        let ci = 2.0 * y as f64 / n as f64 - 1.0;
        let mut byte: i64 = 0;
        let mut bit = 0;
        for x in 0..n {
            let cr = 2.0 * x as f64 / n as f64 - 1.5;
            let (mut zr, mut zi, mut tr, mut ti) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut i = 0;
            while i < iter && tr + ti <= limit {
                zi = 2.0 * zr * zi + ci;
                zr = tr - ti + cr;
                tr = zr * zr;
                ti = zi * zi;
                i += 1;
            }
            byte <<= 1;
            if tr + ti <= limit {
                byte |= 1;
            }
            bit += 1;
            if bit == 8 || x == n - 1 {
                byte <<= 8 - bit;
                checksum = (checksum * 31 + byte) & 0xffffff;
                bytes += 1;
                byte = 0;
                bit = 0;
            }
        }
    }
    println!("{} {} {}", n, bytes, checksum);
}
