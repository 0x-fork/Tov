#!/bin/sh
# Installs Tov (https://tov.sh): builds the compiler from source, from
# https://github.com/richiemcilroy/Tov, and puts `tov` in ~/.cargo/bin.
#
#   curl -fsSL https://tov.sh/install.sh | sh
#
# Needs git and a C compiler (clang or gcc); installs Rust with rustup if it's missing. Run it
# again to update. TOV_DIR sets where the source goes (default ~/.tov/src); TOV_NPM=1 also
# builds Tov's JavaScript engine, for programs that import npm packages (about 10 minutes).
set -eu

say() { printf '%s\n' "$*"; }
start_path=":$PATH:"
fail() { printf 'tov install: %s\n' "$*" >&2; exit 1; }

command -v git > /dev/null 2>&1 || fail "git is needed: install it, then run this again"
if ! command -v cc > /dev/null 2>&1 && ! command -v clang > /dev/null 2>&1 && ! command -v gcc > /dev/null 2>&1; then
    case "$(uname -s)" in
        Darwin) fail "a C compiler is needed: run \`xcode-select --install\`, then run this again" ;;
        *) fail "a C compiler is needed: install gcc or clang (e.g. \`sudo apt install build-essential\`), then run this again" ;;
    esac
fi

if ! command -v cargo > /dev/null 2>&1; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        PATH="$HOME/.cargo/bin:$PATH"
    else
        say "Installing Rust (rustup), which builds Tov's compiler..."
        curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --profile minimal > /dev/null
        PATH="$HOME/.cargo/bin:$PATH"
    fi
fi

# (the compiler needs Rust 1.95 or newer: a rustup install updates, any other says what to do)
rust_minor=$(rustc --version 2>/dev/null | sed -n 's/^rustc 1\.\([0-9]*\).*/\1/p')
if [ -n "$rust_minor" ] && [ "$rust_minor" -lt 95 ]; then
    if command -v rustup > /dev/null 2>&1; then
        say "Updating Rust (Tov needs 1.95 or newer)..."
        rustup update stable -q > /dev/null
    else
        fail "Tov needs Rust 1.95 or newer, and this rustc is 1.$rust_minor: update it (or install rustup from https://rustup.rs), then run this again"
    fi
fi

dir="${TOV_DIR:-$HOME/.tov/src}"
if [ -d "$dir/.git" ]; then
    say "Updating $dir..."
    git -C "$dir" pull -q --ff-only
else
    say "Downloading Tov into $dir..."
    mkdir -p "$(dirname "$dir")"
    git clone -q --depth 1 https://github.com/richiemcilroy/Tov "$dir"
fi

say "Building tov (a minute or two)..."
cargo install -q --locked --path "$dir/crates/tov"

if [ "${TOV_NPM:-}" = 1 ]; then
    say "Building Tov's JavaScript engine, for npm packages (about 10 minutes)..."
    sh "$dir/scripts/jsc/build.sh"
fi

say ""
say "Tov is installed: $(command -v tov 2>/dev/null || echo "$HOME/.cargo/bin/tov")"
case "$start_path" in
    *":$HOME/.cargo/bin:"*) ;;
    *) say "Open a new terminal (or run \`. \"\$HOME/.cargo/env\"\`) so your shell finds tov in ~/.cargo/bin." ;;
esac
say ""
say "  tov run hello.tov        build and run"
say "  tov check --json src/    errors and their fixes, for your agent"
say "  tov build app.tov -o app one native binary"
say ""
say "The language, for you and your agent: https://tov.sh/llms.txt"
