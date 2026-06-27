#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STELLAR_TOML="${STELLAR_TOML:-$ROOT_DIR/stellar.toml}"
CONTRACT_IDS_FILE="${CONTRACT_IDS_FILE:-$ROOT_DIR/.contract-ids.json}"

NETWORK="${NETWORK:-testnet}"
SOURCE="${SOURCE:-default}"
ADMIN="${ADMIN:-}"

POOL_WASM="${ROOT_DIR}/soroban/target/wasm32-unknown-unknown/release/farming_pool.wasm"
FACTORY_WASM="${ROOT_DIR}/soroban/target/wasm32-unknown-unknown/release/factory.wasm"

die() {
  echo "error: $*" >&2
  exit 1
}

run_stellar() {
  local attempt combined last_line
  for attempt in 1 2 3; do
    if combined="$("$@" 2>&1)"; then
      last_line="$(printf '%s\n' "$combined" | awk 'NF { line = $0 } END { print line }')"
      printf '%s' "$last_line"
      return 0
    fi
    if [[ "$combined" == *TxBadSeq* ]] && [[ "$attempt" -lt 3 ]]; then
      echo "warning: transaction sequence mismatch, retrying ($attempt/3)..." >&2
      sleep 3
      continue
    fi
    echo "$combined" >&2
    return 1
  done
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is not installed or not on PATH"
}

toml_value() {
  local section="$1"
  local key="$2"
  awk -v section="[$section]" -v key="$key" '
    $0 == section { in_section = 1; next }
    /^\[/ { in_section = 0 }
    in_section && $1 == key {
      line = $0
      sub(/^[^=]*=[[:space:]]*/, "", line)
      gsub(/"/, "", line)
      print line
      exit
    }
  ' "$STELLAR_TOML"
}

require_cmd stellar

case "$NETWORK" in
  testnet | mainnet) ;;
  *)
    die "NETWORK must be 'testnet' or 'mainnet' (got: $NETWORK)"
    ;;
esac

[[ -f "$STELLAR_TOML" ]] || die "missing $STELLAR_TOML"

RPC_URL="$(toml_value "$NETWORK" "rpc-url")"
NETWORK_PASSPHRASE="$(toml_value "$NETWORK" "network-passphrase")"

[[ -n "$RPC_URL" ]] || die "rpc-url not found for [$NETWORK] in $STELLAR_TOML"
[[ -n "$NETWORK_PASSPHRASE" ]] || die "network-passphrase not found for [$NETWORK] in $STELLAR_TOML"

stellar keys address "$SOURCE" >/dev/null 2>&1 \
  || die "Stellar identity '$SOURCE' not found. Generate one with: stellar keys generate $SOURCE"

if [[ -z "$ADMIN" ]]; then
  ADMIN="$(stellar keys address "$SOURCE")" \
    || die "ADMIN is not set and could not resolve address for SOURCE=$SOURCE"
fi

[[ -f "$POOL_WASM" ]] || die "missing $POOL_WASM — run 'make build' first"
[[ -f "$FACTORY_WASM" ]] || die "missing $FACTORY_WASM — run 'make build' first"

stellar network add "$NETWORK" \
  --rpc-url "$RPC_URL" \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  >/dev/null 2>&1 || true

echo "Installing farming-pool WASM..."
POOL_HASH="$(
  run_stellar stellar contract install \
    --wasm "$POOL_WASM" \
    --network "$NETWORK" \
    --source "$SOURCE" \
    --quiet
)"
sleep 2

[[ -n "$POOL_HASH" ]] || die "failed to install farming-pool WASM (empty hash returned)"

echo "Deploying factory..."
FACTORY_ID="$(
  run_stellar stellar contract deploy \
    --wasm "$FACTORY_WASM" \
    --network "$NETWORK" \
    --source "$SOURCE" \
    --quiet
)"
sleep 2

[[ -n "$FACTORY_ID" ]] || die "failed to deploy factory (empty contract id returned)"

echo "Initializing factory..."
if ! run_stellar stellar contract invoke \
  --id "$FACTORY_ID" \
  --network "$NETWORK" \
  --source "$SOURCE" \
  --quiet \
  -- initialize --admin "$ADMIN" --pool_wasm_hash "$POOL_HASH" >/dev/null; then
  die "failed to initialize factory $FACTORY_ID"
fi

printf '{"factory": "%s", "pool_wasm_hash": "%s", "network": "%s"}\n' \
  "$FACTORY_ID" "$POOL_HASH" "$NETWORK" >"$CONTRACT_IDS_FILE"

echo "Deployment complete."
echo "  factory:        $FACTORY_ID"
echo "  pool_wasm_hash: $POOL_HASH"
echo "  saved to:       $CONTRACT_IDS_FILE"
