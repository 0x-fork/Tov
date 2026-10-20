// The Benchmarks Game k-nucleotide (single-threaded), on a DNA sequence generated in the program
// with fasta's LCG and Homo sapiens frequencies: count every k-mer for k = 1, 2 (printed as sorted
// percentages) and for the lengths of five given k-mers (printed as counts), with a string-keyed
// hash table: std HashMap with keys borrowed from the sequence.

use std::collections::HashMap;

const IM: u32 = 139968;
const IA: u32 = 3877;
const IC: u32 = 29573;

fn frequencies(seq: &[u8], k: usize) -> HashMap<&[u8], i64> {
    let mut counts: HashMap<&[u8], i64> = HashMap::new();
    for key in seq.windows(k) {
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

fn print_frequencies(seq: &[u8], k: usize) {
    let counts = frequencies(seq, k);
    let mut list: Vec<(&[u8], i64)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let total = (seq.len() - k + 1) as f64;
    for (key, count) in list {
        println!("{} {:.3}", std::str::from_utf8(key).unwrap(), 100.0 * count as f64 / total);
    }
    println!();
}

fn print_count(seq: &[u8], fragment: &str) {
    let counts = frequencies(seq, fragment.len());
    println!("{}\t{}", counts.get(fragment.as_bytes()).copied().unwrap_or(0), fragment);
}

fn main() {
    let n: usize = 5_000_000;
    let table = [(b'A', 0.3029549426680), (b'C', 0.1979883004921), (b'G', 0.1975473066391), (b'T', 0.3015094502008)];
    let mut cum = Vec::with_capacity(table.len());
    let mut p = 0.0;
    for &(c, prob) in &table {
        p += prob;
        cum.push((c, p));
    }
    let last = cum.len() - 1;
    let mut seq = Vec::with_capacity(n);
    let mut seed: u32 = 42;
    for _ in 0..n {
        seed = (seed * IA + IC) % IM;
        let r = seed as f64 / IM as f64;
        let mut i = 0;
        while i < last && cum[i].1 <= r {
            i += 1;
        }
        seq.push(cum[i].0);
    }

    print_frequencies(&seq, 1);
    print_frequencies(&seq, 2);
    for fragment in ["GGT", "GGTA", "GGTATT", "GGTATTTTAATT", "GGTATTTTAATTTATAGT"] {
        print_count(&seq, fragment);
    }
}
