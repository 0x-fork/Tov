#!/bin/sh
# Writes scripts/jsc/order-linux.txt: the engine's and runtime's functions that Tov programs run as
# they start and serve, in the order lld lays them out (`tov build` passes it to lld for programs
# linking Tov's JavaScriptCore). Packed together, what starting touches of the engine's code is a
# few MB of pages, not most of its 27 MB in the kernel's 64 KB fault-around windows.
#
# Linux, with valgrind (callgrind), curl and lld; run from the repository after `bun install` in
# bench/npm and bench/cap-media's run.py (its out/ms). Startup's functions come first.
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
T=$root/target/release/tov
jsc=${TOV_JSC_DIR:-$HOME/.cache/tov/jsc/linux-x86_64}
w=$(mktemp -d)
VG="valgrind --tool=callgrind --dump-instr=no --compress-strings=no --demangle=no"

# serve NAME BINARY PORT PATH: start it, request PATH until it answers, then 30 more times
serve() {
    PORT=$3 $VG --callgrind-out-file="$w/$1.out" "$2" >/dev/null 2>"$w/$1.err" &
    vp=$!
    for i in $(seq 1 240); do curl -s -o /dev/null "http://127.0.0.1:$3$4" && break; sleep 0.5; done
    for i in $(seq 1 30); do curl -s -o /dev/null "http://127.0.0.1:$3$4"; done
    callgrind_control -d $vp >/dev/null 2>&1 || true
    sleep 2
    kill $vp; wait $vp 2>/dev/null || true
}

cd "$root/bench/npm"
$T build -g startup.tov -o "$w/startup" >/dev/null
$T build -g service.tov -o "$w/service" >/dev/null
$VG --callgrind-out-file="$w/startup.out" "$w/startup" >/dev/null 2>"$w/startup.err"
PORT=39411 $VG --callgrind-out-file="$w/service.out" "$w/service" >/dev/null 2>"$w/service.err" &
vp=$!
for i in $(seq 1 240); do curl -s -o /dev/null http://127.0.0.1:39411/ && break; sleep 0.5; done
for i in $(seq 1 30); do curl -s -X POST -d '{"name":"Ada","email":"ada@example.com","age":36}' http://127.0.0.1:39411/ >/dev/null; done
curl -s -X POST -d 'nope' http://127.0.0.1:39411/ >/dev/null
callgrind_control -d $vp >/dev/null 2>&1 || true
sleep 2
kill $vp; wait $vp 2>/dev/null || true
if [ -f "$root/bench/cap-media/out/ms/main.tov" ]; then
    cd "$root/bench/cap-media/out/ms"
    $T build -g main.tov -o "$w/ms" >/dev/null
    NODE_ENV=production MEDIA_SERVER_WEBHOOK_SECRET=x serve ms "$w/ms" 39413 /health
fi
cd "$root/bench/http"
$T build -g server.tov.ts -o "$w/http" >/dev/null
serve http "$w/http" 39414 /

# (only the engine's and runtime's own symbols: a program's generated ones differ program to program)
( nm --defined-only "$jsc/lib/libtovjsc.a" 2>/dev/null
  for o in "$HOME"/.cache/tov/c/rt-*.o "$HOME"/.cache/tov/c/js-*.o "$HOME"/.cache/tov/c/node-*.o "$HOME"/.cache/tov/c/napi-*.o "$HOME"/.cache/tov/c/re*-*.o; do
      nm --defined-only "$o" 2>/dev/null || true
  done ) | awk 'NF >= 3 {print $3}' | sort -u > "$w/known.txt"
python3 - "$w" "$here/order-linux.txt" <<'E'
import glob, re, sys
w, out = sys.argv[1], sys.argv[2]
known = set(open(f"{w}/known.txt").read().split())
order, seen = [], set()
for path in [f"{w}/startup.out"] + sorted(p for p in glob.glob(f"{w}/*.out*") if "startup" not in p):
    obj = None
    for line in open(path, errors="replace"):
        if line.startswith("ob="):
            obj = line[3:].strip()
        elif line.startswith("fn="):
            fn = re.sub(r"'\d+$", "", line[3:].strip())
            if obj and obj.startswith(w) and fn in known and fn not in seen:
                seen.add(fn)
                order.append(fn)
open(out, "w").write("\n".join(order) + "\n")
print(f"{out}: {len(order)} functions")
E
rm -rf "$w"
