// Benchmarks Game pidigits on num-bigint: the first N digits of pi.
use num_bigint::BigInt;
use num_traits::{One, ToPrimitive, Zero};
use std::fmt::Write;

const N: u32 = 10000;

fn main() {
    let mut acc = BigInt::zero();
    let mut den = BigInt::one();
    let mut num = BigInt::one();
    let (mut k, mut i) = (0u32, 0u32);
    let mut line = String::new();
    let mut out = String::new();
    while i < N {
        k += 1;
        let k2 = BigInt::from(k * 2 + 1);
        acc = (acc + &num * 2u32) * &k2;
        den *= &k2;
        num *= k;
        if num > acc {
            continue;
        }
        let d3 = (&num * 3u32 + &acc) / &den;
        let d4 = (&num * 4u32 + &acc) / &den;
        if d3 != d4 {
            continue;
        }
        let d = d3.to_u32().unwrap();
        line.push(char::from(b'0' + d as u8));
        i += 1;
        if i % 10 == 0 {
            let _ = writeln!(out, "{line}\t:{i}");
            line.clear();
        }
        acc = (acc - &den * d) * 10u32;
        num *= 10u32;
    }
    print!("{out}");
}
