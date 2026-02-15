#!/usr/bin/env bash
set -euo pipefail

ROOT="/Users/mr.j/myRoom/code/ai/codexProxy"
PORT="8889"
MODE="${1:-}"

if [ -z "${MODE}" ]; then
  echo "Select proxy mode:"
  echo "1) anthropic"
  echo "2) openai"
  echo "q) quit"
  while true; do
    read -r -p "Choice: " CHOICE
    case "${CHOICE}" in
      1|anthropic|claude)
        MODE="anthropic"
        break
        ;;
      2|openai|openai-compatible|openai_compatible)
        MODE="openai"
        break
        ;;
      q|Q|quit|exit)
        exit 0
        ;;
      *)
        echo "Invalid choice, try again."
        ;;
    esac
  done
fi

case "${MODE}" in
  anthropic|claude)
    TARGET="${ROOT}/codex-proxy-anthropic.js"
    ;;
  openai|openai-compatible|openai_compatible)
    TARGET="${ROOT}/codex-proxy-openai-compatible.js"
    ;;
  *)
    echo "Unknown mode: ${MODE}"
    echo "Usage: $0 [anthropic|openai]"
    exit 1
    ;;
esac

if [ ! -f "${TARGET}" ]; then
  echo "Proxy file not found: ${TARGET}"
  exit 1
fi

if command -v lsof >/dev/null 2>&1; then
  PIDS="$(lsof -ti "tcp:${PORT}" 2>/dev/null || true)"
else
  echo "lsof not found, skip port check."
  PIDS=""
fi

if [ -n "${PIDS}" ]; then
  echo "Port ${PORT} is in use, stopping PID(s): ${PIDS}"
  for PID in ${PIDS}; do
    kill -TERM "${PID}" 2>/dev/null || true
  done

  for _ in 1 2 3 4 5; do
    sleep 0.2
    PIDS="$(lsof -ti "tcp:${PORT}" 2>/dev/null || true)"
    if [ -z "${PIDS}" ]; then
      break
    fi
  done

  if [ -n "${PIDS}" ]; then
    echo "Port ${PORT} still in use, forcing stop PID(s): ${PIDS}"
    for PID in ${PIDS}; do
      kill -KILL "${PID}" 2>/dev/null || true
    done
  fi
fi

echo "Starting: ${TARGET}"
exec node "${TARGET}"
