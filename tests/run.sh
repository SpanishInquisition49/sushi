#!/bin/sh
# Everything that can be checked without the shell: Rust tests (unit + end-to-end with a real
# daemon and hook), clippy, the Luau scripts (syntax, Noctalia's lint, behaviour in stand-ins)
# the desktop app's scripts and icons, and the sounds. Run from anywhere; needs cargo, luajit and noctalia (for the lint).
set -e
cd "$(dirname "$0")/.."
echo "== cargo test";   cargo test 2>&1 | grep -E "^test result|FAILED|panicked" | grep -v "0 passed" || true
cargo test >/dev/null 2>&1 || { echo "cargo test FAILED"; cargo test 2>&1 | tail -30; exit 1; }
echo "== clippy";       test -z "$(cargo clippy --all-targets 2>&1 | grep -E '^(warning|error)')" && echo "clean"
echo "== luau syntax";  for f in plugin/*.luau plugin/lib/*.luau; do luajit -bl "$f" >/dev/null || { echo "syntax error in $f"; exit 1; }; done; echo ok
if command -v noctalia >/dev/null 2>&1; then echo "== noctalia lint"; noctalia plugins lint plugin | tail -1; fi
for t in tests/lua/*_test.lua; do echo "== $t"; luajit "$t"; done
if command -v node >/dev/null 2>&1; then echo "== app ui (syntax)"; for f in app/ui/js/*.js; do node --check "$f" || { echo "syntax error in $f"; exit 1; }; done; echo ok; fi
echo "== icons";        python3 tools/make_icon.py --check && echo ok
echo "== sounds";       python3 tools/make_sounds.py --check | tail -1
echo "ALL GOOD"
