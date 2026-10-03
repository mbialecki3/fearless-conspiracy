#!/usr/bin/env bash
set -euo pipefail

cargo build
smoke_root="$(mktemp -d /tmp/terminal-salem-smoke.XXXXXX)"
socket_path="$smoke_root/game.sock"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
  fi
  case "$smoke_root" in
    /tmp/terminal-salem-smoke.*) rm -r -- "$smoke_root" ;;
  esac
}
trap cleanup EXIT

target/debug/salem-server "$socket_path" --preset demo5 --scenario --fast --allow-same-uid \
  >"$smoke_root/server.log" 2>&1 &
server_pid=$!
for _ in {1..20}; do
  [[ -S "$socket_path" ]] && break
  sleep 0.05
done
[[ -S "$socket_path" ]]

( sleep 0.4; echo /ready; sleep 0.8; echo '/investigate 4'; sleep 7 ) \
  | target/debug/salem-client "$socket_path" Alice --plain >"$smoke_root/alice.log" 2>&1 & p1=$!
sleep 0.05
( sleep 0.4; echo /ready; sleep 0.8; echo '/heal 1'; sleep 7 ) \
  | target/debug/salem-client "$socket_path" Bob --plain >"$smoke_root/bob.log" 2>&1 & p2=$!
sleep 0.05
( sleep 0.4; echo /ready; sleep 0.8; echo '/watch 1'; sleep 7 ) \
  | target/debug/salem-client "$socket_path" Cara --plain >"$smoke_root/cara.log" 2>&1 & p3=$!
sleep 0.05
( sleep 0.4; echo /ready; sleep 0.8; echo '/kill 1'; sleep 7 ) \
  | target/debug/salem-client "$socket_path" Drew --plain >"$smoke_root/drew.log" 2>&1 & p4=$!
sleep 0.05
( sleep 0.4; echo /ready; sleep 7 ) \
  | target/debug/salem-client "$socket_path" Emi --plain >"$smoke_root/emi.log" 2>&1 & p5=$!

wait "$server_pid"
server_pid=""
for client_pid in "$p1" "$p2" "$p3" "$p4" "$p5"; do
  wait "$client_pid"
done

grep -q 'appears suspicious' "$smoke_root/alice.log"
grep -q 'protected target #1 was attacked' "$smoke_root/bob.log"
grep -q 'visitors to #1: #2, #4' "$smoke_root/cara.log"
grep -q 'attack on #1 failed' "$smoke_root/drew.log"
grep -q 'Final faction result: Draw' "$smoke_root/emi.log"
echo "five-client IPC game smoke test passed"

