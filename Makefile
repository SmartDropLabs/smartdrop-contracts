.PHONY: build test deploy-testnet deploy-mainnet

build:
	cd soroban && cargo build --target wasm32-unknown-unknown --release

test:
	cd soroban && cargo test --workspace

deploy-testnet:
	NETWORK=testnet ./scripts/deploy.sh

deploy-mainnet:
	NETWORK=mainnet ./scripts/deploy.sh
