# zcash-devtool (Tachyon Fork💫)

The Tachyon fork of [zcash-devtool](https://github.com/zcash/zcash-devtool). See the upstream repo for docs and usage info.

Here you will find utilities for interacting with the Tachyon shielded protocol.

## Build

`RUSTFLAGS='--cfg zcash_unstable="nutachyon"' cargo build --release --locked --features tachyon`

the binary will be available in `./target/release/zcash-devtool`

Instructions for the Tachyon Alpha Testnet.

## Init

```sh
DEVTOOL="$PWD/target/release/zcash-devtool"
WALLET="$HOME/tachyon-wallet"

"$DEVTOOL" tachyon --wallet-dir "$WALLET" init \
  --rpc-url http://tachyscan.cnode.phd:18232 \
  --activation-height 8 --accept-mock-proofs
"$DEVTOOL" tachyon --wallet-dir "$WALLET" address
```

## Sync

Sync downloads and validates chain data to update your wallet's note status and balances.

```sh
"$DEVTOOL" tachyon --wallet-dir "$WALLET" sync --max-blocks 10000
```

Repeat until `caught_up` is `true`.

## Receive (Tachyon)

Get funds from the [faucet](https://tachyscan.cnode.phd/faucet) using your `tachyon_address`, and save the downloaded note JSON as `~/Downloads/faucet-note.json`.

```sh
"$DEVTOOL" tachyon --wallet-dir "$WALLET" receive \
  --receipt "$HOME/Downloads/faucet-note.json"
"$DEVTOOL" tachyon --wallet-dir "$WALLET" sync --max-blocks 10000
"$DEVTOOL" tachyon --wallet-dir "$WALLET" balance
```

The payment must be mined before it becomes available.

## Send (Tachyon)

```sh
"$DEVTOOL" tachyon --wallet-dir "$WALLET" send \
  --to 'tachyon-test:RECIPIENT_PAYMENT_KEY' --amount-zat 1000000
"$DEVTOOL" tachyon --wallet-dir "$WALLET" export-receipt \
  --txid PAYMENT_TXID --out "$HOME/payment.json"
```

Replace `PAYMENT_TXID` with the ID returned by `send`. Share `payment.json` privately with the recipient so they can import it with `receive`.

## License

All code in this workspace is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
