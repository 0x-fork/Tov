// The "1 billion nested loop iterations" program: a[i] += j % u over 10,000 x 100,000.
// u and r come from the argument count at runtime, so they can't be constant-folded.

fn main() {
    let u = 40 + (std::env::args().len() as u32 - 1);
    let mut state = u as i64;
    for _ in 0..3 {
        state = (state * 16807) % 2147483647;
    }
    let r = (state % 10000) as usize;
    let mut a = vec![0u32; 10_000];
    for i in 0..10_000 {
        for j in 0..100_000u32 {
            a[i] += j % u;
        }
        a[i] += r as u32;
    }
    println!("{}", a[r]);
}
