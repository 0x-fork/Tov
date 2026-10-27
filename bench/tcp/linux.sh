#!/bin/sh
# Runs the TCP benchmark on Linux (epoll) in Tov's build container (scripts/linux.sh): Tov,
# Rust (tokio) and Bun (from the oven/bun image).
#   bench/tcp/linux.sh [run.py args...]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
bin="$here/out/linux"
mkdir -p "$bin"
# (fetched again when the pinned version changes)
bun=1.4.4
[ -x "$bin/bun" ] && [ "$(cat "$bin/bun.version" 2> /dev/null)" = "$bun" ] || {
    docker run --rm -v "$bin:/out" "oven/bun:$bun" cp /usr/local/bin/bun /out/bun && echo "$bun" > "$bin/bun.version"
}
exec "$root/scripts/linux.sh" sh -c '
  set -e
  command -v pgrep > /dev/null || (apt-get update -qq && apt-get install -y -qq procps > /dev/null)
  cargo build --release -q
  tov build bench/tcp/server.tov.ts -o bench/tcp/out/linux/tov-server > /dev/null
  tov build bench/tcp/client.tov.ts -o bench/tcp/out/linux/tov-client > /dev/null
  cargo build --release -q --manifest-path bench/tcp/rust/Cargo.toml --target-dir /target/tcpbench
  cp /target/tcpbench/release/server bench/tcp/out/linux/rust-server
  cp /target/tcpbench/release/load bench/tcp/out/linux/rust-load
  cd bench/tcp && BIN_DIR=/tov/bench/tcp/out/linux python3 run.py --no-build --only tov,bun,rust,rust-mt "$@"
' sh "$@"
