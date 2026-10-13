//! The load generator: `load [-c conns] [-t threads] [-d secs] [-w warmup] [-s size] [-P depth] [--tls ca.pem] [-p port]`.
//! Each OS thread runs a current-thread tokio runtime driving its share of the connections. A
//! connection keeps `depth` messages of `size` bytes in flight and counts one done each time
//! that many bytes have come back; latency runs from a message's send to the last byte of its
//! echo. Prints one JSON object: messages/s, MB/s each way, latency percentiles (µs).
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

struct Opts { conns: usize, threads: usize, secs: f64, warm: f64, size: usize, depth: usize, port: u16, ca: Option<String> }

fn parse() -> Opts {
    let a: Vec<String> = std::env::args().collect();
    let mut o = Opts { conns: 64, threads: 4, secs: 5.0, warm: 1.0, size: 64, depth: 1, port: 3300, ca: None };
    let mut i = 1;
    while i < a.len() {
        let v = a.get(i + 1).cloned().unwrap_or_default();
        match a[i].as_str() {
            "-c" => o.conns = v.parse().unwrap(),
            "-t" => o.threads = v.parse().unwrap(),
            "-d" => o.secs = v.parse().unwrap(),
            "-w" => o.warm = v.parse().unwrap(),
            "-s" => o.size = v.parse().unwrap(),
            "-P" => o.depth = v.parse().unwrap(),
            "-p" => o.port = v.parse().unwrap(),
            "--tls" => o.ca = Some(std::fs::read_to_string(&v).unwrap()),
            _ => panic!("unknown option {}", a[i]),
        }
        i += 2;
    }
    o
}

// phase: 0 warming up, 1 measuring, 2 stopped
async fn drive<S: AsyncRead + AsyncWrite + Unpin>(s: S, size: usize, depth: usize, phase: Arc<AtomicU8>) -> (u64, Vec<u32>) {
    let (mut r, mut w) = tokio::io::split(s);
    let msg = vec![b'x'; size];
    let mut buf = vec![0u8; 65536];
    let mut sent = std::collections::VecDeque::with_capacity(depth);
    let mut done = 0u64;
    let mut lat = Vec::new();
    let mut batch = Vec::with_capacity(size * depth);
    for _ in 0..depth {
        batch.extend_from_slice(&msg);
        sent.push_back(Instant::now());
    }
    if w.write_all(&batch).await.is_err() {
        return (0, lat);
    }
    let mut got = 0usize;
    loop {
        let n = match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        got += n;
        let mut k = 0;
        while got >= size {
            got -= size;
            let t0 = sent.pop_front().unwrap();
            let p = phase.load(Ordering::Relaxed);
            if p == 1 {
                done += 1;
                lat.push(t0.elapsed().as_micros().min(u32::MAX as u128) as u32);
            }
            k += 1;
        }
        if phase.load(Ordering::Relaxed) == 2 {
            break;
        }
        if k > 0 {
            batch.clear();
            for _ in 0..k {
                batch.extend_from_slice(&msg);
                sent.push_back(Instant::now());
            }
            if w.write_all(&batch).await.is_err() {
                break;
            }
        }
    }
    (done, lat)
}

fn main() {
    let o = parse();
    let phase = Arc::new(AtomicU8::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let connector = o.ca.as_ref().map(|ca| tcpbench::client_tls(ca));
    let mut handles = Vec::new();
    for t in 0..o.threads {
        let n = o.conns / o.threads + usize::from(t < o.conns % o.threads);
        let (phase, connector, size, depth, port) = (phase.clone(), connector.clone(), o.size, o.depth, o.port);
        handles.push(std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let mut tasks = Vec::new();
                for _ in 0..n {
                    let s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                    s.set_nodelay(true).unwrap();
                    let phase = phase.clone();
                    match &connector {
                        Some(c) => {
                            let name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost").unwrap();
                            let t = c.connect(name, s).await.unwrap();
                            if std::env::var("SHOW_CIPHER").is_ok() {
                                eprintln!("{:?} {:?}", t.get_ref().1.protocol_version(), t.get_ref().1.negotiated_cipher_suite());
                            }
                            tasks.push(tokio::spawn(drive(t, size, depth, phase)));
                        }
                        None => tasks.push(tokio::spawn(drive(s, size, depth, phase))),
                    }
                }
                let mut done = 0;
                let mut lat = Vec::new();
                for t in tasks {
                    let (d, l) = t.await.unwrap();
                    done += d;
                    lat.extend(l);
                }
                (done, lat)
            })
        }));
    }
    let _ = stop;
    std::thread::sleep(Duration::from_secs_f64(o.warm));
    phase.store(1, Ordering::Relaxed);
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs_f64(o.secs));
    phase.store(2, Ordering::Relaxed);
    let elapsed = t0.elapsed().as_secs_f64();
    let mut done = 0u64;
    let mut lat: Vec<u32> = Vec::new();
    for h in handles {
        let (d, l) = h.join().unwrap();
        done += d;
        lat.extend(l);
    }
    lat.sort_unstable();
    let pct = |p: f64| if lat.is_empty() { 0 } else { lat[((lat.len() as f64 * p) as usize).min(lat.len() - 1)] };
    let avg = if lat.is_empty() { 0.0 } else { lat.iter().map(|&x| x as f64).sum::<f64>() / lat.len() as f64 };
    let mps = done as f64 / elapsed;
    println!(
        "{{\"msgs\": {done}, \"mps\": {mps:.1}, \"mbps\": {:.2}, \"avg_us\": {avg:.1}, \"p50_us\": {}, \"p99_us\": {}, \"p999_us\": {}}}",
        mps * o.size as f64 / 1e6,
        pct(0.5),
        pct(0.99),
        pct(0.999)
    );
}
