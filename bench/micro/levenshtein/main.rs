// Levenshtein distance (two-row DP, byte comparisons) between every pair of N words
// generated with a Park-Miller LCG. Prints the minimum distance and the sum of all distances.

fn levenshtein(s: &str, t: &str) -> usize {
    let (mut a, mut b) = (s.as_bytes(), t.as_bytes());
    if a.len() > b.len() {
        std::mem::swap(&mut a, &mut b);
    }
    let m = a.len();
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for j in 1..=b.len() {
        curr[0] = j;
        let bj = b[j - 1];
        for i in 1..=m {
            let cost = if a[i - 1] == bj { 0 } else { 1 };
            curr[i] = (prev[i] + 1).min(curr[i - 1] + 1).min(prev[i - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

fn main() {
    let alphabet = b"abcdefgh";
    let count = 800;
    let mut state: i64 = 42;
    let mut words: Vec<String> = Vec::with_capacity(count);
    for _ in 0..count {
        state = (state * 16807) % 2147483647;
        let len = 20 + (state % 41) as usize;
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            state = (state * 16807) % 2147483647;
            s.push(alphabet[(state % 8) as usize] as char);
        }
        words.push(s);
    }
    let mut min_dist = usize::MAX;
    let mut total = 0;
    for i in 0..count {
        for j in i + 1..count {
            let d = levenshtein(&words[i], &words[j]);
            min_dist = min_dist.min(d);
            total += d;
        }
    }
    println!("{} {}", min_dist, total);
}
