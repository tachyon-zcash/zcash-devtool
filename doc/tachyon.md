# Experimental Tachyon wallets

The optional `tachyon` command group supports direct Zakura RPC synchronization,
shielding a transparent UTXO, sending Tachyon payments with change, unshielding,
and transparent-to-transparent transfers. Each wallet has its own persistent state and machine-readable JSON
output. A later script or sidecar can operate several wallets; there is no
random workload loop, miner, or automatic aggregation in devtool.

**Private development networks and test funds only.** Ragu proofs are mocked.
Wallet seeds and note receipts are **unencrypted**, unlike ordinary devtool
wallets. On Unix, wallet directories must have mode 0700 and newly created
database/receipt files have mode 0600. On other platforms, restrict access with
OS ACLs yourself. Back up the private wallet directory, not just the seed: note
plaintexts cannot currently be recovered from the chain. Never commit it.

## Build and node requirements

Use Rust 1.97.1 (pinned in `rust-toolchain.toml`):

```sh
ROCKSDB_LIB_DIR="/opt/homebrew/lib" CARGO_INCREMENTAL=0 \
  RUSTFLAGS='--cfg zcash_unstable="nutachyon"' \
  cargo build --release --locked --features tachyon
```

This feature pins Tachyon `a4abfbdd`, Ragu `0881656d`, and Zakura common
`a9081dc0`, matching the experimental node integration. It uses common's V7
serialization, txid and sighash implementation, including a zero ZIP-233
amount. The ordinary wallet dependencies remain separate and unchanged.

Connect to a trusted, Tachyon-enabled archive Zakura node with
`gettachyonblock` (the proof-sync RPC), `getaddressutxos` with `chainInfo: true`,
and `sendrawtransaction`. NuTachyon must
already be active. The node and wallet must use the same epoch length: the
default build uses 4096 blocks, not Tachyon's short `test-epoch-size` setting.
The supplied activation height is checked against the feed. Genesis and the
activation block are pinned so accidental network changes/regenesis fail closed.

For the internal miner, set `tachyon_workload = false`: the old output-only
generator replaces normal mempool selection. Keep `internal_miner = true` and
set `miner_address` to a wallet's `funding_address` if you want its block rewards
to fund the simulation. Wait for 100 confirmations before shielding coinbase
outputs. The RPC also needs retained previous transaction bodies for shielding.
No datastore reset is required.

Keep RPC on localhost, an SSH tunnel, or an authenticated HTTPS connection.
Use `--cookie-file /path/to/.cookie` during initialization if cookie auth is
enabled. The cookie is reread on each request. An authenticated HTTP connection
does not encrypt credentials; do not expose it on an untrusted network.

## Two-wallet example

Commands below use the compiled binary; no Cargo rebuild is needed per payment.
Replace height `8` with your node's NuTachyon activation height.

```sh
DEVTOOL=./target/release/zcash-devtool

$DEVTOOL tachyon --wallet-dir ./alice init \
  --rpc-url http://127.0.0.1:18232 --activation-height 8 --accept-mock-proofs
$DEVTOOL tachyon --wallet-dir ./bob init \
  --rpc-url http://127.0.0.1:18232 --activation-height 8 --accept-mock-proofs

$DEVTOOL tachyon --wallet-dir ./alice address
$DEVTOOL tachyon --wallet-dir ./alice sync
$DEVTOOL tachyon --wallet-dir ./alice balance
```

`sync` ingests at most 1000 blocks per invocation (adjustable with
`--max-blocks`, maximum 10000). Repeat until `caught_up` is true. Balances are
offline snapshots of the last successful sync, with note states `unconfirmed`,
`available`, `reserved`, and `spent`. Top-level `available_zat` remains the
**Tachyon-only** balance, before fees, and is not a promise that multiple notes
can be combined.

`balance` also reports a separate `transparent` object for this wallet's
`funding_address` (not addresses belonging to the ordinary `wallet` commands):

- `total_zat`: all confirmed unspent funds at the transparent snapshot.
- `available_zat`: confirmed, mature funds not reserved by this wallet.
- `immature_zat`: unreserved coinbase funds with fewer than 100 confirmations.
- `reserved_zat`: confirmed UTXOs reserved for a pending shielding transaction,
  including one whose broadcast failed ambiguously.
- `utxo_count`, `address`, `synced_height`, and `synced_hash`: snapshot metadata.

The three balance categories are mutually exclusive and sum to `total_zat`.
Available funds are before fees; each UTXO must individually cover the shielding
fee. Unconfirmed incoming funds are excluded. The snapshot does not account for
other software's mempool spends; `shield` rechecks the selected output with the
node before spending it.

Transparent funds are refreshed only when `sync` catches up to the UTXO index's
captured tip, so both pools describe the same chain. A partial sync retains the
older transparent snapshot and its explicit height. Before the first caught-up
sync, or after a reorg invalidates that snapshot during a partial sync,
`transparent` is `null`, **not zero**. RPC errors leave the previous committed
state intact. `balance` never contacts the node. Existing wallet databases are
upgraded automatically on their next caught-up sync; no reset is needed.

Fund Alice's reported transparent `funding_address`, then shield an exact amount
in zatoshis. The fixed 50,000-zatoshi fee is charged **in addition** to the amount:

```sh
# Automatically select one eligible UTXO, shielding exactly 1,000,000 zatoshis:
$DEVTOOL tachyon --wallet-dir ./alice shield 1000000
# Or choose the funding outpoint explicitly:
$DEVTOOL tachyon --wallet-dir ./alice shield 1000000 --txid PREVIOUS_TXID --index 0
# Omit the amount to shield that output's entire value minus the fee (as before):
$DEVTOOL tachyon --wallet-dir ./alice shield --txid PREVIOUS_TXID --index 0
# Wait until mined, then:
$DEVTOOL tachyon --wallet-dir ./alice sync
$DEVTOOL tachyon --wallet-dir ./alice balance
```

Automatic selection picks the smallest eligible confirmed UTXO covering amount
plus fee, excluding this wallet's reservations and outputs unavailable in the
node's mempool view. It does not combine UTXOs. `--txid` and `--index` must be
provided together; bare `shield` requires an amount or explicit outpoint.
Any surplus returns to the wallet's funding address as **transparent change**,
not an extra Tachyon note. Change must confirm before it can be selected again.

**Coinbase exception:** Zakura normally forbids transparent outputs in a
transaction spending a coinbase reward. The wallet therefore accepts a coinbase
UTXO only if the amount equals its entire value minus the fee (or the amount is
omitted with an explicit outpoint). It never silently shields extra funds.
To shield mining rewards of an arbitrary size, use the explicit full-output
command instead; custom-network relaxations of this rule are not auto-detected.

Pay Bob using his `tachyon_address`. The current `tachyon-test:<payment-key>`
format is a devtool-specific test encoding, not a standardized Zcash address.

```sh
$DEVTOOL tachyon --wallet-dir ./alice send \
  --to tachyon-test:BOBS_PAYMENT_KEY --amount-zat 1000000
# Use the returned transaction ID:
$DEVTOOL tachyon --wallet-dir ./alice export-receipt \
  --txid PAYMENT_TXID --out ./payment.json
$DEVTOOL tachyon --wallet-dir ./bob receive --receipt ./payment.json
# After mining:
$DEVTOOL tachyon --wallet-dir ./bob sync
$DEVTOOL tachyon --wallet-dir ./bob balance

$DEVTOOL tachyon --wallet-dir ./bob unshield \
  --to TESTNET_TRANSPARENT_ADDRESS --amount-zat 100000
```

After an unshielded output confirms, it can circulate transparently without a
Tachyon bundle or any proof generation:

```sh
$DEVTOOL tachyon --wallet-dir ./alice transparent-send \
  --to BOBS_FUNDING_ADDRESS --amount-zat 100000 --operation-id round-12:alice
# After mining, Bob discovers the payment by syncing; no receipt import is needed.
$DEVTOOL tachyon --wallet-dir ./bob sync
```

`transparent-send` uses the smallest eligible confirmed **non-coinbase** P2PKH
UTXO, pays the recipient exactly `--amount-zat`, charges the additional 50,000-zatoshi
fee, and returns any remainder to the sender's funding address. P2PKH and P2SH
testnet recipients are supported. It does not combine inputs or spend unconfirmed
change. Coinbase rewards must be shielded first, even if an output would fund the
transfer exactly. Use the same wallet directories as the other `tachyon` commands,
not the separate ordinary `wallet` command group.

**Deliver receipts confidentially out of band.** They contain the recipient's
note plaintext, not a spending key, but expose the amount and commitment
opening. Tachyon does not currently provide an on-chain encrypted-note scanning
format here. Import checks ownership and network identity, and does not credit
funds until the note commitment appears in the validated feed. Own outputs and
change are recorded automatically. Export never overwrites an existing file.

## Recovery and proof updates

### Durable operation IDs for a sidecar

All four payment commands (`shield`, `send`, `unshield`, `transparent-send`)
accept optional `--operation-id ID`. IDs are unique **within a wallet**, and must
contain 1–128 ASCII letters, digits, `-`, `_`, `.`, or `:`.

```sh
$DEVTOOL tachyon --wallet-dir ./alice send \
  --to tachyon-test:BOBS_PAYMENT_KEY --amount-zat 100000 --operation-id round-13:alice
# Offline lookup, including after a timeout or process restart:
$DEVTOOL tachyon --wallet-dir ./alice operation round-13:alice
```

The ID and original request are committed atomically with signed transaction
bytes, input reservation, and own note receipts **before broadcast**. Repeating
the same ID and payment arguments returns the original transaction with
`reused: true`, without contacting the node, creating another payment, or
rebroadcasting. Different arguments with the same ID fail. Operation records
remain bound to the original transaction after mining, reorgs, or expiry; a
deliberate replacement requires a new ID. Explicit shielding txids are compared
case-insensitively. Other request fields must match exactly.

Lookup reports `txid`, the original `request`, `has_receipt`, `expiry_height`,
and `status` (`pending`, `mined`, or `expired`) at the displayed `synced_height`.
`pending` means saved but not observed mined; it does **not** confirm mempool
acceptance. Run `sync` to refresh status, then `submit --txid TXID` to resend the
same bytes when needed. A repeated expired operation returns `status: expired`
and must not be interpreted as a successful new payment. `transactions` also
includes each transaction's operation ID, or `null` when none was supplied.

For crash recovery, the sidecar should persist its ID and request before invoking
devtool. After an interrupted call, repeat that exact request with that ID. If
the first attempt never committed, a payment is created; otherwise the original
is returned. Deliver any Tachyon receipt using the recovered txid. Do not change
IDs merely because broadcasting timed out. Existing v1 wallet databases gain the
operation table lazily; no reset is required. No keys or receipt plaintexts are
included in the operation response.

### Reservations and proof synchronization

Payments consume one sufficiently large note, create recipient/change output
actions, and pay 50,000 zatoshis. Transparent funding uses one owned testnet
P2PKH UTXO; unshielding and transparent sending support testnet P2PKH or P2SH recipients. Multi-note
selection/consolidation, encrypted seed storage, memo transport and ordinary
`wallet`/`inspect` integration are not implemented.

Input reservation, raw transaction and output receipts are committed to SQLite
**before** broadcast. If a request fails, the input remains reserved because
the node may have accepted the transaction. Recover using:

```sh
$DEVTOOL tachyon --wallet-dir ./alice transactions
$DEVTOOL tachyon --wallet-dir ./alice submit --txid PAYMENT_TXID
# Export still works even if the original broadcast failed.
$DEVTOOL tachyon --wallet-dir ./alice export-receipt \
  --txid PAYMENT_TXID --out ./recovered-payment.json
```

Retry resends identical bytes. Reservations expire only after the synced tip
passes the transaction expiry height. Payments expire within 20 blocks. Payments
with Tachyon data never expire beyond the current epoch; at an epoch's last block,
wait for its successor before creating one. Transparent-only payments have no
Tachyon epoch restriction. A reorganization rolls back stored public inputs; note and
transaction states are recalculated, including mined transactions returning to
pending. Concurrent commands against one wallet fail with a busy error rather
than racing input selection; parallelize across wallets instead.

The wallet validates ordered stamp commitments, anchor continuity and epoch
crossings from `gettachyonblock`. It reconstructs mock note-secret, nullifier,
closed-epoch absence and spendability proofs locally, then lifts the spending
stamp to its synced tip within the active epoch. It supports empty epochs and
retains historical public inputs past node tachygram pruning. It does **not**
independently validate consensus or the authenticity of a malicious RPC node.

This implementation rebuilds proofs and loads its stored feed on demand. It is
for small development chains, not a scalable proof service. Closed-epoch evidence
routes individual stamps into bounded QR buckets before merging them, retaining
every stamp's anchor transition even when its selected bucket is empty. Note
membership and nullifier absence may use different buckets covering the same
epoch. Each polynomial stays within Ragu's capacity (8,191 tachygrams with the
pinned dependency), including when mock proofs are enabled. The wallet's separate
resource budget is 65,536 tachygrams and stamps per epoch; larger epochs fail
explicitly and need a more scalable evidence builder. Existing wallet databases
and public inputs can be reused; this does not require a chain reset.
A later sidecar can cache evidence, schedule wallets,
exchange receipts, and observe mining; node-side aggregation remains Zakura's job.

## Validation

```sh
ROCKSDB_LIB_DIR="/opt/homebrew/lib" CARGO_INCREMENTAL=0 \
  RUSTFLAGS='--cfg zcash_unstable="nutachyon"' \
  RUSTDOCFLAGS='--cfg zcash_unstable="nutachyon"' \
  cargo test --locked -p tachyon-wallet
ROCKSDB_LIB_DIR="/opt/homebrew/lib" CARGO_INCREMENTAL=0 \
  RUSTFLAGS='--cfg zcash_unstable="nutachyon"' \
  cargo clippy --locked --workspace --all-targets --features tachyon -- -D warnings
```
