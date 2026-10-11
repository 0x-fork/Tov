// 40,000! on num-bigint, then its decimal digits.
use num_bigint::BigUint;
use num_traits::One;

const N: u32 = 40000;

fn main() {
    let mut f = BigUint::one();
    for i in 2..=N {
        f *= i;
    }
    let s = f.to_string();
    let sum: u64 = s.bytes().map(|c| (c - b'0') as u64).sum();
    let zeros = s.bytes().rev().take_while(|&c| c == b'0').count();
    println!("{N}! has {} digits", s.len());
    println!("digit sum {sum}, trailing zeros {zeros}");
}
