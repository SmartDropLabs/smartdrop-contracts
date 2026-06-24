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

Scaffold on-chain code with the **Stellar CLI** (`stellar contract init`) when you are ready, then publish factory and pool contract IDs to the frontend repo’s environment variables.
