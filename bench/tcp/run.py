#!/usr/bin/env python3
"""TCP echo benchmark: Tov (Bun.listen) vs Bun vs Node (node:net) vs Rust (tokio), plain and TLS.

    bench/tcp/run.py [--conns 64] [--threads 8] [--secs 5] [--sizes 64,4096] [--depth 1] [--tls] [--only tov,bun]

Builds every server, then starts one at a time and drives it with rust/target/release/load (each
connection keeps `depth` messages in flight and waits for their echo). Servers are interleaved
across repetitions and the median is reported, with the server's CPU per message and memory.
Writes bench/results/tcp-<timestamp>.json.
"""
import argparse, json, os, re, shutil, socket, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(HERE, "out")
PORT = 3310
BIN = os.environ.get("BIN_DIR")


def sh(cmd, **kw):
    subprocess.run(cmd, check=True, **kw)


def build():
    os.makedirs(OUT, exist_ok=True)
    sh(["cargo", "build", "--release", "-q", "--manifest-path", os.path.join(ROOT, "Cargo.toml")])
    sh([os.path.join(ROOT, "target/release/tov"), "build", os.path.join(HERE, "server.tov.ts"), "-o", os.path.join(OUT, "tov-server")], stdout=subprocess.DEVNULL)
    sh([os.path.join(ROOT, "target/release/tov"), "build", os.path.join(HERE, "client.tov.ts"), "-o", os.path.join(OUT, "tov-client")], stdout=subprocess.DEVNULL)
    sh(["cargo", "build", "--release", "-q", "--manifest-path", os.path.join(HERE, "rust/Cargo.toml")])


def rust_bin(name):
    return os.path.join(BIN, f"rust-{name}") if BIN else os.path.join(HERE, "rust/target/release", name)


SERVERS = {
    "tov": lambda: [os.path.join(BIN or OUT, "tov-server")],
    "bun": lambda: [os.path.join(BIN, "bun") if BIN else "bun", os.path.join(HERE, "server.bun.ts")],
    "node": lambda: ["node", os.path.join(HERE, "server.node.mjs")],
    "rust": lambda: [rust_bin("server")],      # tokio, current-thread (one core, as Bun and Tov)
    "rust-mt": lambda: [rust_bin("server")],   # tokio's multi-thread runtime, every core
}


def wait_port(port, timeout=10):
    end = time.time() + timeout
    while time.time() < end:
        try:
            socket.create_connection(("127.0.0.1", port), 0.2).close()
            return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("server did not start")


def cpu_seconds(pgid):
    """Total CPU time (user+sys) of the server's process group."""
    pids = subprocess.run(["pgrep", "-g", str(pgid)], capture_output=True, text=True).stdout.split()
    if not pids:
        return 0.0
    if os.path.exists("/proc/self/stat"):  # Linux: clock ticks (procps `ps` shows whole seconds)
        tick, total = os.sysconf("SC_CLK_TCK"), 0.0
        for pid in pids:
            try:
                fields = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
                total += (int(fields[11]) + int(fields[12])) / tick
            except OSError:
                pass
        return total
    out = subprocess.run(["ps", "-o", "time=", "-p", ",".join(pids)], capture_output=True, text=True).stdout
    total = 0.0
    for t in out.split():
        parts = [float(x) for x in t.replace("-", ":").split(":")]
        total += sum(v * 60 ** i for i, v in enumerate(reversed(parts)))
    return total


def pss_mb(pgid):
    """Proportional set size of the process group (MB): pages shared between processes (a
    forked worker's code and libraries) are split between them. Linux only; else RSS."""
    pids = subprocess.run(["pgrep", "-g", str(pgid)], capture_output=True, text=True).stdout.split()
    total, ok = 0, False
    for pid in pids:
        try:
            for line in open(f"/proc/{pid}/smaps_rollup"):
                if line.startswith("Pss:"):
                    total += int(line.split()[1])
                    ok = True
        except OSError:
            pass
    if ok:
        return total / 1024
    # macOS: the sum of physical footprints (a process's own dirty memory; like PSS, a forked
    # worker's shared code isn't counted again)
    if pids and shutil.which("footprint"):
        out = subprocess.run(["footprint", "-f", "bytes"] + [a for p in pids for a in ("-p", p)], capture_output=True, text=True).stdout
        # Several processes: per-process lines, then their sum as "Summary Footprint".
        total = re.search(r"Summary Footprint: (\d+) B", out)
        if total:
            return int(total.group(1)) / (1 << 20)
        fp = [int(m.group(1)) for m in re.finditer(r"Footprint: (\d+) B", out)]
        if fp:
            return sum(fp) / (1 << 20)
    return rss_mb(pgid)


def rss_mb(pgid):
    """Resident memory of the server's process group (MB)."""
    pids = subprocess.run(["pgrep", "-g", str(pgid)], capture_output=True, text=True).stdout.split()
    if not pids:
        return 0.0
    out = subprocess.run(["ps", "-o", "rss=", "-p", ",".join(pids)], capture_output=True, text=True).stdout
    return sum(int(x) for x in out.split()) / 1024



CLIENTS = {
    "tov": lambda o: [os.path.join(BIN or OUT, "tov-client")],
    "bun": lambda o: [os.path.join(BIN, "bun") if BIN else "bun", os.path.join(HERE, "client.bun.ts")],
    "node": lambda o: ["node", os.path.join(HERE, "client.node.mjs")],
    # the load generator on one thread: tokio's current-thread runtime, one core like the others
    "rust": lambda o: [rust_bin("load"), "-p", str(PORT), "-t", "1", "-c", str(o.conns), "-s", str(o.size), "-d", str(o.secs), "-w", "1"],
}


def run_client(name, a, size):
    """One client against the fixed echo server: its messages/s, CPU per message and memory."""
    class O: pass
    o = O(); o.conns, o.size, o.secs = a.conns, size, a.secs
    env = dict(os.environ, PORT=str(PORT), CONNS=str(a.conns), SIZE=str(size), SECS=str(a.secs))
    p = subprocess.Popen(CLIENTS[name](o), env=env, stdout=subprocess.PIPE, text=True, start_new_session=True)
    time.sleep(1.1)
    c0 = cpu_seconds(p.pid)
    time.sleep(a.secs - 0.2)
    c1 = cpu_seconds(p.pid)
    mem = pss_mb(p.pid)
    out = p.communicate()[0]
    r = json.loads(out.strip().splitlines()[-1])
    r["pss_mb"] = mem
    r["cpu_us_per_msg"] = (c1 - c0) * 1e6 / max(1.0, r["mps"] * (a.secs - 0.2))
    r["cpu_cores"] = (c1 - c0) / (a.secs - 0.2)
    return r


def clients(a):
    names = a.only.split(",")
    names = [n for n in names if n in CLIENTS]
    sizes = [int(s) for s in a.sizes.split(",")]
    env = dict(os.environ, PORT=str(PORT), THREADS="8")
    server = subprocess.Popen([rust_bin("server")], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    samples = {}
    try:
        wait_port(PORT)
        for rep in range(a.repeat):
            for name in names:
                for size in sizes:
                    samples.setdefault((name, size), []).append(run_client(name, a, size))
                    time.sleep(0.3)
            print(f"  repetition {rep + 1}/{a.repeat} done", file=sys.stderr, flush=True)
    finally:
        os.killpg(server.pid, 15)
        server.wait()
    results = []
    print(f"\n== TCP clients: {a.conns} connections to a Rust echo server (8 threads), one message in flight each ==")
    print(f"{'client':<8} {'size':>6} {'msgs/s':>10} {'cores':>6} {'cpu/msg':>8} {'mem':>7}")
    for size in sizes:
        for name in names:
            runs = sorted(samples[(name, size)], key=lambda x: x["mps"])
            r = dict(runs[len(runs) // 2])
            r["mps_all"] = [round(x["mps"]) for x in runs]
            r.update(client=name, size=size, conns=a.conns)
            results.append(r)
            print(f"{name:<8} {size:>6} {r['mps']:>10,.0f} {r['cpu_cores']:>6.2f} {r['cpu_us_per_msg']:>6.2f}us {r['pss_mb']:>5.1f}MB", flush=True)
    return results


def load(conns, threads, secs, size, depth, tls, pgid):
    warm = 1.0
    cmd = [rust_bin("load"), "-p", str(PORT), "-c", str(conns), "-t", str(threads), "-d", str(secs), "-w", str(warm), "-s", str(size), "-P", str(depth)]
    if tls:
        cmd += ["--tls", os.path.join(HERE, "ca.pem")]
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, text=True)
    time.sleep(warm + 0.1)
    c0 = cpu_seconds(pgid)
    time.sleep(secs - 0.2)
    c1 = cpu_seconds(pgid)
    mem, pss = rss_mb(pgid), pss_mb(pgid)
    r = json.loads(p.communicate()[0])
    r["rss_mb"], r["pss_mb"] = mem, pss
    r["cpu_us_per_msg"] = (c1 - c0) * 1e6 / max(1.0, r["mps"] * (secs - 0.2))
    r["cpu_cores"] = (c1 - c0) / (secs - 0.2)
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--conns", type=int, default=64)
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--secs", type=float, default=5)
    ap.add_argument("--sizes", default="64,4096")
    ap.add_argument("--depth", type=int, default=1)
    ap.add_argument("--tls", action="store_true")
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--only", default="tov,bun,node,rust,rust-mt")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--clients", action="store_true", help="measure the client side instead (Bun.connect & co.)")
    a = ap.parse_args()
    if not a.no_build:
        build()
    if a.clients:
        results = clients(a)
        tag = "-linux" if os.uname().sysname == "Linux" else ""
        path = os.path.join(HERE, "../results", time.strftime(f"tcp-clients{tag}-%Y%m%d-%H%M%S.json"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            json.dump({"machine": f"{os.uname().sysname} {os.uname().machine}", "results": results}, f, indent=1)
        print(f"\nwrote {os.path.relpath(path, ROOT)}")
        return 0
    names = a.only.split(",")
    sizes = [int(s) for s in a.sizes.split(",")]
    samples = {}
    for rep in range(a.repeat):
        for name in names:
            env = dict(os.environ, PORT=str(PORT), TLS="1" if a.tls else "", CERT_DIR=HERE, THREADS="8" if name == "rust-mt" else "1")
            proc = subprocess.Popen(SERVERS[name](), env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            try:
                wait_port(PORT)
                time.sleep(0.3)
                for size in sizes:
                    samples.setdefault((name, size), []).append(load(a.conns, a.threads, a.secs, size, a.depth, a.tls, proc.pid))
            finally:
                os.killpg(proc.pid, 15)
                proc.wait()
                time.sleep(0.5)
        print(f"  repetition {rep + 1}/{a.repeat} done", file=sys.stderr, flush=True)
    results = []
    kind = "TLS" if a.tls else "TCP"
    print(f"\n== {kind} echo, {a.conns} connections, {a.depth} message(s) in flight each ==")
    print(f"{'server':<8} {'size':>6} {'msgs/s':>10} {'MB/s':>8} {'p50':>8} {'p99':>8} {'p99.9':>8} {'cores':>6} {'cpu/msg':>8} {'mem':>7}")
    for size in sizes:
        for name in names:
            runs = sorted(samples[(name, size)], key=lambda x: x["mps"])
            r = dict(runs[len(runs) // 2])
            r["mps_all"] = [round(x["mps"]) for x in runs]
            r.update(server=name, size=size, conns=a.conns, depth=a.depth, tls=a.tls)
            results.append(r)
            print(f"{name:<8} {size:>6} {r['mps']:>10,.0f} {r['mbps']:>8.1f} {r['p50_us']:>6}us {r['p99_us']:>6}us {r['p999_us']:>6}us {r['cpu_cores']:>6.2f} {r['cpu_us_per_msg']:>6.2f}us {r['pss_mb']:>5.1f}MB", flush=True)
    os.makedirs(os.path.join(HERE, "../results"), exist_ok=True)
    tag = "-linux" if os.uname().sysname == "Linux" else ""
    path = os.path.join(HERE, "../results", time.strftime(f"tcp{tag}-%Y%m%d-%H%M%S.json"))
    with open(path, "w") as f:
        json.dump({"machine": f"{os.uname().sysname} {os.uname().machine}", "results": results}, f, indent=1)
    print(f"\nwrote {os.path.relpath(path, ROOT)}")


if __name__ == "__main__":
    sys.exit(main())
