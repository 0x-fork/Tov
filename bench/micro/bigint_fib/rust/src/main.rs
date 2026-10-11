// The 300,000th Fibonacci number by repeated addition on num-bigint.
use num_bigint::BigUint;
use num_traits::{One, Zero};

const N: u32 = 300000;

fn main() {
    let mut a = BigUint::zero();
    let mut b = BigUint::one();
    for _ in 0..N {
        let t = &a + &b;
        a = std::mem::replace(&mut b, t);
    }
    let s = a.to_string();
    println!("fib({N}) has {} digits", s.len());
    println!("first 20: {}", &s[..20]);
    println!("last 20: {}", &s[s.len() - 20..]);
}
