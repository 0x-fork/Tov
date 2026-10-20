// The Benchmarks Game fasta (single-threaded): an ALU repeat and two sequences drawn from
// nucleotide tables with the LCG, written as 60-column lines. Each line goes to a sink that
// counts lines and bytes and checksums the bytes instead of writing them to stdout.

const WIDTH: usize = 60;
const IM: u32 = 139968;
const IA: u32 = 3877;
const IC: u32 = 29573;

const ALU: &[u8] = b"GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGG\
GAGGCCGAGGCGGGCGGATCACCTGAGGTCAGGAGTTCGAGA\
CCAGCCTGGCCAACATGGTGAAACCCCGTCTCTACTAAAAAT\
ACAAAAATTAGCCGGGCGTGGTGGCGCGCGCCTGTAATCCCA\
GCTACTCGGGAGGCTGAGGCAGGAGAATCGCTTGAACCCGGG\
AGGCGGAGGTTGCAGTGAGCCGAGATCGCGCCACTGCACTCC\
AGCCTGGGCGACAGAGCGAGACTCCGTCTCAAAAA";

struct Sink {
    lines: u64,
    bytes: u64,
    hash: u32,
}

impl Sink {
    // One line: its bytes, then '\n'.
    fn write_line(&mut self, line: &[u8]) {
        let mut h = self.hash;
        for &b in line {
            h = (h * 31 + b as u32) & 0xffffff;
        }
        self.hash = (h * 31 + b'\n' as u32) & 0xffffff;
        self.bytes += line.len() as u64 + 1;
        self.lines += 1;
    }
}

struct Rng {
    seed: u32,
}

impl Rng {
    fn next(&mut self) -> f64 {
        self.seed = (self.seed * IA + IC) % IM;
        self.seed as f64 / IM as f64
    }
}

fn repeat_fasta(out: &mut Sink, header: &str, src: &[u8], mut n: usize) {
    out.write_line(header.as_bytes());
    let len = src.len();
    let mut s = src.to_vec();
    s.extend_from_slice(&src[..WIDTH]);
    let mut pos = 0;
    while n > 0 {
        let w = n.min(WIDTH);
        out.write_line(&s[pos..pos + w]);
        pos += w;
        if pos >= len {
            pos -= len;
        }
        n -= w;
    }
}

fn random_fasta(out: &mut Sink, rng: &mut Rng, header: &str, table: &[(u8, f64)], mut n: usize) {
    out.write_line(header.as_bytes());
    let mut cum = Vec::with_capacity(table.len());
    let mut p = 0.0;
    for &(c, prob) in table {
        p += prob;
        cum.push((c, p));
    }
    let last = cum.len() - 1;
    let mut line = [0u8; WIDTH];
    while n > 0 {
        let w = n.min(WIDTH);
        for slot in &mut line[..w] {
            let r = rng.next();
            let mut i = 0;
            while i < last && cum[i].1 <= r {
                i += 1;
            }
            *slot = cum[i].0;
        }
        out.write_line(&line[..w]);
        n -= w;
    }
}

fn main() {
    let n: usize = 10_000_000;
    let iub = [
        (b'a', 0.27),
        (b'c', 0.12),
        (b'g', 0.12),
        (b't', 0.27),
        (b'B', 0.02),
        (b'D', 0.02),
        (b'H', 0.02),
        (b'K', 0.02),
        (b'M', 0.02),
        (b'N', 0.02),
        (b'R', 0.02),
        (b'S', 0.02),
        (b'V', 0.02),
        (b'W', 0.02),
        (b'Y', 0.02),
    ];
    let homo_sapiens = [(b'a', 0.3029549426680), (b'c', 0.1979883004921), (b'g', 0.1975473066391), (b't', 0.3015094502008)];
    let mut out = Sink { lines: 0, bytes: 0, hash: 0 };
    let mut rng = Rng { seed: 42 };
    repeat_fasta(&mut out, ">ONE Homo sapiens alu", ALU, 2 * n);
    random_fasta(&mut out, &mut rng, ">TWO IUB ambiguity codes", &iub, 3 * n);
    random_fasta(&mut out, &mut rng, ">THREE Homo sapiens frequency", &homo_sapiens, 5 * n);
    println!("{} {} {}", out.lines, out.bytes, out.hash);
}
