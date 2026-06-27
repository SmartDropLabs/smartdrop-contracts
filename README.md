# SmartDrop contracts

[![CI](https://github.com/SmartDropLabs/smartdrop-contracts/actions/workflows/ci.yml/badge.svg)](https://github.com/SmartDropLabs/smartdrop-contracts/actions/workflows/ci.yml)

**Soroban (Rust)** smart contracts and design notes for [**SmartDrop**](https://github.com/SmartDropLabs/SmartDrop) on **Stellar**.

## Related repositories

| Repository | Role |
|------------|------|
| [**smart-frontend**](https://github.com/SmartDropLabs/smart-frontend) | Next.js app (Freighter, RPC) |
| [**smartdrop-backend**](https://github.com/SmartDropLabs/smartdrop-backend) | APIs and indexing (planned) |
| [**SmartDrop**](https://github.com/SmartDropLabs/SmartDrop) | Original monorepo (reference) |

## Layout

- [`contracts/`](./contracts/README.md) — reserved layout / pointers  
- [`soroban/`](./soroban/README.md) — Soroban design and future Rust workspace

Contract sources live under [`soroban/`](./soroban/README.md). After deployment, publish the factory contract ID and RPC URL to the frontend repo’s environment variables.

## Quick Start (deployment)

### Prerequisites

- [Rust](https://rustup.rs/) with the `wasm32-unknown-unknown` target
- [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools) (`stellar`)
- A funded Stellar identity on the target network (testnet or mainnet)

```bash
rustup target add wasm32-unknown-unknown
stellar keys generate default --network testnet   # if you do not already have an identity
```

Fund testnet accounts via [Friendbot](https://friendbot.stellar.org/?addr=YOUR_ADDRESS).

Network RPC URLs and passphrases are defined in [`stellar.toml`](./stellar.toml). The deploy script registers the selected network with the CLI automatically.

### Build, test, and deploy

```bash
make build            # compile release WASM to soroban/target/wasm32-unknown-unknown/release/
make test             # run contract unit tests

make deploy-testnet   # install pool WASM, deploy factory, initialize, write .contract-ids.json
make deploy-mainnet   # same flow on mainnet (requires a funded mainnet identity)
```

`scripts/deploy.sh` accepts optional environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `NETWORK` | `testnet` | `testnet` or `mainnet` |
| `SOURCE` | `default` | Stellar CLI identity used to sign transactions |
| `ADMIN` | address of `SOURCE` | Factory admin passed to `initialize` |
| `STELLAR_TOML` | `./stellar.toml` | Network configuration file |
| `CONTRACT_IDS_FILE` | `./.contract-ids.json` | Output path for deployed IDs |

Example with a custom identity:

```bash
SOURCE=alice ADMIN=G... make deploy-testnet
```

On success, `.contract-ids.json` contains the factory contract ID and installed pool WASM hash (gitignored).
