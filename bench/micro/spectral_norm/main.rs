// The Benchmarks Game spectral-norm (single-threaded): power iteration on an infinite matrix.

// The denominator of A[i][j] = 1 / ((i + j) * (i + j + 1) / 2 + i + 1).
fn a(i: usize, j: usize) -> f64 {
    ((i + j) * (i + j + 1) / 2 + i + 1) as f64
}

fn multiply_av(v: &[f64], av: &mut [f64]) {
    for (i, out) in av.iter_mut().enumerate() {
        *out = v.iter().enumerate().map(|(j, &x)| x / a(i, j)).sum();
    }
}

fn multiply_atv(v: &[f64], atv: &mut [f64]) {
    for (i, out) in atv.iter_mut().enumerate() {
        *out = v.iter().enumerate().map(|(j, &x)| x / a(j, i)).sum();
    }
}

fn multiply_atav(v: &[f64], tmp: &mut [f64], out: &mut [f64]) {
    multiply_av(v, tmp);
    multiply_atv(tmp, out);
}

fn main() {
    let n = 5500;
    let mut u = vec![1.0; n];
    let mut v = vec![0.0; n];
    let mut tmp = vec![0.0; n];
    for _ in 0..10 {
        multiply_atav(&u, &mut tmp, &mut v);
        multiply_atav(&v, &mut tmp, &mut u);
    }
    let vbv: f64 = u.iter().zip(&v).map(|(x, y)| x * y).sum();
    let vv: f64 = v.iter().map(|x| x * x).sum();
    println!("{:.9}", (vbv / vv).sqrt());
}
