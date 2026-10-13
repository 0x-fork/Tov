//! The echo server, as tokio's own example writes it: every byte read is written back.
//! PORT, TLS=1 (cert.pem and key.pem from CERT_DIR), THREADS (1: tokio's current-thread
//! runtime, one core like Bun and Tov; more: the multi-thread runtime).
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

async fn echo<S: AsyncRead + AsyncWrite + Unpin>(mut s: S) {
    let mut buf = vec![0u8; 65536];
    loop {
        match s.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if s.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        }
    }
}

async fn serve() {
    let port = tcpbench::env("PORT", "3300");
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
    let tls = if tcpbench::env("TLS", "") == "1" {
        let dir = tcpbench::env("CERT_DIR", ".");
        let cert = std::fs::read_to_string(format!("{dir}/cert.pem")).unwrap();
        let key = std::fs::read_to_string(format!("{dir}/key.pem")).unwrap();
        Some(tcpbench::server_tls(&cert, &key))
    } else {
        None
    };
    loop {
        let (s, _) = listener.accept().await.unwrap();
        s.set_nodelay(true).unwrap();
        let tls = tls.clone();
        tokio::spawn(async move {
            match tls {
                Some(acceptor) => {
                    if let Ok(t) = acceptor.accept(s).await {
                        echo(t).await
                    }
                }
                None => echo(s).await,
            }
        });
    }
}

fn main() {
    let threads: usize = tcpbench::env("THREADS", "1").parse().unwrap();
    let rt = if threads <= 1 {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    } else {
        tokio::runtime::Builder::new_multi_thread().worker_threads(threads).enable_all().build().unwrap()
    };
    rt.block_on(serve());
}
