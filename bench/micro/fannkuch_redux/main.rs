// The Benchmarks Game fannkuch-redux (single-threaded): count pancake flips over every permutation.

fn fannkuch(n: usize) -> (i32, i32) {
    let mut perm = [0usize; 16];
    let mut perm1 = [0usize; 16];
    let mut count = [0usize; 16];
    let mut max_flips = 0;
    let mut checksum = 0;
    let mut perm_count = 0;
    for (i, p) in perm1[..n].iter_mut().enumerate() {
        *p = i;
    }
    let mut r = n;
    loop {
        while r != 1 {
            count[r - 1] = r;
            r -= 1;
        }
        perm[..n].copy_from_slice(&perm1[..n]);
        let mut flips = 0;
        loop {
            let k = perm[0];
            if k == 0 {
                break;
            }
            // A swap loop: faster than `perm[..=k].reverse()` for these short prefixes.
            let (mut i, mut j) = (0, k);
            while i < j {
                perm.swap(i, j);
                i += 1;
                j -= 1;
            }
            flips += 1;
        }
        max_flips = max_flips.max(flips);
        checksum += if perm_count % 2 == 0 { flips } else { -flips };
        loop {
            if r == n {
                return (checksum, max_flips);
            }
            perm1[..=r].rotate_left(1);
            count[r] -= 1;
            if count[r] > 0 {
                break;
            }
            r += 1;
        }
        perm_count += 1;
    }
}

fn main() {
    let n = 11;
    let (checksum, max_flips) = fannkuch(n);
    println!("{}\nPfannkuchen({}) = {}", checksum, n, max_flips);
}
