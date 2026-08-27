#!/usr/bin/env bash
# Rebuild and run Lysilogy for local development: the Rust backend (cargo run
# -- serve) and the Vite dev server (frontend, with hot reload) side by side.
# Vite proxies /api to the backend on 127.0.0.1:7320 and serves the frontend on
# http://localhost:7319/, which this script opens automatically.
set -euo pipefail

FRONTEND_URL="http://localhost:7319/"
BACKEND_BIND="127.0.0.1:7320"

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

echo "==> Starting backend (cargo run -- serve --bind $BACKEND_BIND)"
cargo run -- serve --bind "$BACKEND_BIND" &
backend_pid=$!

echo "==> Starting frontend (npm run dev)"
(cd web && npm run dev) &
frontend_pid=$!

if command -v xdg-open >/dev/null 2>&1; then
  echo "==> Opening frontend at $FRONTEND_URL"
  (sleep 1 && xdg-open "$FRONTEND_URL" >/dev/null 2>&1) &
else
  echo "==> xdg-open not found; open $FRONTEND_URL manually"
fi

wait -n "$backend_pid" "$frontend_pid"
