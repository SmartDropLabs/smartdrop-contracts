# SmartDrop on Stellar (Soroban)

On-chain logic for SmartDrop targets **[Soroban](https://soroban.stellar.org/)** smart contracts written in **Rust** on the **Stellar** network.

## Intended design

- A **factory** contract deploys or registers isolated **farming pool** instances per campaign.
- Pools accept **Stellar assets** (classic or token contracts) for **locks**; participants accrue **airdrop credits** over time from locked amount × duration × configurable rates.
- Use the **Stellar Asset Contract (SAC)** or Soroban token interfaces for trustline-style or contract-held balances, per your issuance model.

## Tooling

- [Soroban CLI](https://developers.stellar.org/docs/tools/developer-tools) (`stellar` / `soroban`)
- Rust + `soroban-sdk` for contract code
- Deploy to **Futurenet** or **Testnet**, then set `NEXT_PUBLIC_FACTORY_CONTRACT_ID` and `NEXT_PUBLIC_SOROBAN_RPC_URL` in the Next.js app

Contract sources are not checked in yet; add a `contracts/` Rust crate here when you scaffold with `stellar contract init` and wire the front end to `invoke` / simulation responses.
