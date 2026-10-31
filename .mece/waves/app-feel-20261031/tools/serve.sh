#!/usr/bin/env bash
# Usage: tools/serve.sh <port> <static-dist-dir>
# Runs the native backend on a throwaway data dir (USER DATA rule) and serves the given UI build.
set -euo pipefail
PORT=$1; DIST=$(realpath "$2"); REPO=/home/v9i/Projects/Kala-Agent
T=$(mktemp -d /tmp/kala-look-$PORT-XXXX)
export HOME=$T KALA_AGENT_DATA_DIR=$T/ka KALA_STUDIO_DATA_DIR=$T/ks XDG_DATA_HOME=$T/xdg KALA_STUDIO_TOKEN=opus-look-token-0123456789abcdef
setsid "$REPO/studio/backend-native/kala-studio" --host 127.0.0.1 --port "$PORT" --data-dir "$T/ks" --static-dist "$DIST" >"$T/backend.log" 2>&1 &
for _ in $(seq 1 40); do curl -sf "http://127.0.0.1:$PORT/api/studio/health" >/dev/null && break; sleep 0.5; done
echo "serving $DIST on http://127.0.0.1:$PORT (temp dir $T; stop with: pkill -f -- '--port $PORT'; rm -rf $T)"
