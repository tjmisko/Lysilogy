#!/usr/bin/env bash
# Rebuild and run Lysilogy for local development: the Rust backend (cargo run
# -- serve) and the Vite dev server (frontend, with hot reload) side by side.
# Vite proxies /api to the backend on 127.0.0.1:7319; open http://127.0.0.1:5173.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [ ! -d web/node_modules ]; then
  echo "==> Installing frontend dependencies"
  (cd web && npm install)
fi

echo "==> Building backend (cargo build)"
cargo build

backend_pid=""
frontend_pid=""

cleanup() {
  trap - INT TERM EXIT
  [ -n "$frontend_pid" ] && kill "$frontend_pid" 2>/dev/null
  [ -n "$backend_pid" ] && kill "$backend_pid" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup INT TERM EXIT

echo "==> Starting backend (cargo run -- serve)"
cargo run -- serve &
backend_pid=$!

echo "==> Starting frontend (npm run dev)"
(cd web && npm run dev) &
frontend_pid=$!

wait -n "$backend_pid" "$frontend_pid"
