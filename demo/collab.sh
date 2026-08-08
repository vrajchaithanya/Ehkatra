#!/usr/bin/env bash
# demo/collab.sh — BOOTSTRAP row 10's visible proof (POSIX hosts).
#
# Identical to demo/collab.ps1: relay on loopback 7423, two peers editing one
# workbook concurrently, pass iff their state hashes match.
#
# Two terminals, to watch it live:
#   Terminal 1:  cargo run --release --bin ehkatra-relay
#   Terminal 2:  cargo run --release --bin ehkatra-peer -- 1 alice
#   Terminal 3:  cargo run --release --bin ehkatra-peer -- 2 bob
set -euo pipefail
cd "$(dirname "$0")/.."

echo "building..."
cargo build --release --bin ehkatra-relay --bin ehkatra-peer

out="demo/out"
mkdir -p "$out"

echo
echo "starting relay on 127.0.0.1:7423"
./target/release/ehkatra-relay >"$out/relay.log" 2>&1 &
relay_pid=$!
trap 'kill "$relay_pid" 2>/dev/null || true' EXIT
sleep 0.4

echo "starting two peers"
./target/release/ehkatra-peer 1 alice >"$out/alice.log" 2>&1 &
alice_pid=$!
./target/release/ehkatra-peer 2 bob >"$out/bob.log" 2>&1 &
bob_pid=$!
wait "$alice_pid" "$bob_pid"

echo
echo "--- alice ---"; cat "$out/alice.log"
echo
echo "--- bob ---";   cat "$out/bob.log"

alice_hash=$(sed -n 's/^STATE HASH [0-9]*: //p' "$out/alice.log")
bob_hash=$(sed -n 's/^STATE HASH [0-9]*: //p' "$out/bob.log")

echo
if [ -z "$alice_hash" ] || [ -z "$bob_hash" ]; then
  echo "DEMO FAILED — a peer produced no state hash"
  exit 1
fi
if [ "$alice_hash" != "$bob_hash" ]; then
  echo "DEMO FAILED — replicas diverged"
  echo "  alice: $alice_hash"
  echo "  bob:   $bob_hash"
  exit 1
fi
echo "DEMO PASS — both replicas converged"
echo "  state hash: $alice_hash"
