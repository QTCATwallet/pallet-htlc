# pallet-htlc

Native hash-time-locked transfers for the Quantus chain, implementing the
[`pallet_htlc` proposal (QIP_HTLC, draft v0.1)](#design-rationale-aligned-with-the-qip). It lets QTC be
swapped trustlessly against any chain that already has HTLCs (every EVM chain, TRON, Bitcoin):
no settlement bots, no custodian, no admin key.

Written against **Quantus-Network/chain @ `482c5b9e02bec0adc70797eade0a67f60baf2619`**
(2026-09-21): the chain's vendored `frame-support` 45.1.0, `frame-system` 45.0.0 and
`pallet-balances` 46.0.0, crates.io `sp-core` 39 / `sp-io` 44 / `sp-runtime` 45, and the chain's
own `Cargo.lock`. It follows the chain's pallet layout (`lib.rs`, `mock.rs`, `tests.rs`,
`weights.rs`, `benchmarking.rs`) and `.rustfmt.toml`.

## Swap flow

1. Seller picks a random 32-byte secret `S`, publishes `H = sha256(S)`.
2. Buyer locks USDT for the seller on the EVM side under `H`, expiring at `T_usdt`.
3. Seller calls `Htlc::lock(buyer, amount, H, T_qtc)` on Quantus. Buyer verifies it on chain.
4. Seller claims the USDT by revealing `S` on the EVM side.
5. Anyone (buyer, relayer, watcher) calls `Htlc::claim(id, S)` on Quantus; the QTC goes to the buyer.
6. If anything stalls, each side refunds itself after its own expiry (`Htlc::refund(id)`).

## Interface

| Call | Who | Requires | Effect |
|---|---|---|---|
| `lock(recipient, #[compact] amount, hashlock: [u8;32], expiry: BlockNumber)` (index 0) | signed sender | `amount >= MinLock`; `now + MinDuration <= expiry <= now + MaxDuration`; sender's open locks `< MaxLocksPerAccount`; enough free balance for `amount + LockDeposit` while keeping ED | holds `amount` under `HoldReason::HtlcLock` and `LockDeposit` under `HoldReason::HtlcDeposit`; emits `Locked { id, sender, recipient, amount, hashlock, expiry }` |
| `claim(id: H256, preimage: [u8;32])` (index 1) | **anyone** (signed) | `now < expiry`, `sha256(preimage) == hashlock` | held principal → `recipient` (`transfer_on_hold`), deposit released to sender, lock removed; emits `Claimed { id, preimage }` |
| `refund(id: H256)` (index 2) | **anyone** (signed) | `now >= expiry` | principal and deposit released to sender, lock removed; emits `Refunded { id }` |

Lock id: `blake2_256(SCALE(sender, recipient, amount, hashlock, expiry, nonce))`, where `nonce` is
the sender's per-account counter (`Htlc::Nonces`, incremented on every lock). Callers can predict it
(`Pallet::lock_id`) or read it from the `Locked` event.

Storage:

- `Locks: map H256 => { sender, recipient, amount, hashlock, expiry, deposit }` (`Identity` hasher —
  keys are already blake2 outputs)
- `LockCount: map AccountId => u32` (open locks per sender)
- `Nonces: map AccountId => u64`

Errors: `AmountTooLow`, `ExpiryTooSoon`, `ExpiryTooLate`, `TooManyLocks`, `LockExists`,
`LockNotFound`, `InvalidPreimage`, `LockExpired`, `LockNotExpired` (plus `Balances` token errors if
the sender can't cover the hold).

No admin/root path, no amend, no cancel before expiry, no partial claim.

## Design rationale (aligned with the QIP)

- **SHA-256 over exactly 32 bytes.** `claim` takes `[u8; 32]` and checks `sp_io::hashing::sha2_256`,
  the same value as Solidity `sha256(abi.encodePacked(bytes32 preimage))` in `QubiHTLC.sol`. A fixed
  length on both chains removes the length-mismatch attack.
- **Claim / refund callable by anyone.** The recipient may have no QTC for fees; a relayer can
  finish the swap. Funds can only go to `recipient` (claim) or `sender` (refund).
- **Disjoint windows.** Claimable for `now < expiry`, refundable for `now >= expiry`; the two never
  overlap.
- **Holds, not a pallet pot.** Principal and deposit stay in the sender's account under two named
  holds, so accounting is per-lock and `transfer_all` can't touch them.
- **Claim can't be blocked by the sender.** The claim uses `transfer_on_hold(..., Precision::BestEffort,
  Restriction::Free, Fortitude::Force)`. `Force` means a freeze or balance lock placed on the sender
  *after* locking can't make the claim fail. (With `Polite`, a seller could reveal `S` on the EVM
  side, block the Quantus claim and refund after expiry.) Quantus has no freeze users today, so this
  only guards against future pallets. There is a test for it.
- **Can't get stuck.** `integrity_test` enforces `MinLock >= ExistentialDeposit`, so a claim can
  always create a fresh recipient account. Refund and deposit return use `release` with
  `BestEffort`, which ignores freezes and cannot kill the account. If the released amount ever
  differs (only possible if some other code touched these hold reasons), it is logged instead of
  reverting, so the lock still clears.
- **Self-lock** (`recipient == sender`): claim uses `release` instead of `transfer_on_hold`, the
  same pattern as `pallet_reversible_transfers`, so no self-directed `TransferOnHold` event is
  emitted.
- **Expiry in blocks**, bounded by `MinDuration` / `MaxDuration`. Swap software must keep
  `T_qtc - T_usdt` larger than Quantus finality plus a safety margin, and only treat a claim as final
  at the finalized head. Note that `QubiHTLC.sol` uses **timestamps** (`claim` allowed while
  `block.timestamp <= timeout`), so the wallet has to convert `T_usdt` to a Quantus block height
  conservatively (12 s target block time, PoW variance).

## Integrating into the Quantus runtime

1. Copy this crate to `pallets/htlc`, add `"pallets/htlc"` to `[workspace] members` and
   `pallet-htlc = { path = "./pallets/htlc", default-features = false }` to
   `[workspace.dependencies]`.
2. In `pallets/htlc/Cargo.toml`, replace the pinned git deps with `workspace = true`, e.g.
   `frame-support.workspace = true`, `sp-io.workspace = true`,
   `pallet-balances = { workspace = true, features = ["std"] }`. Use
   `authors/edition/homepage/repository.workspace = true`, add `[lints] workspace = true`, and
   delete the `[patch.crates-io]` block and the copied `Cargo.lock` / `.rustfmt.toml`.
3. Runtime `Cargo.toml`: add `pallet-htlc`, and `pallet-htlc/std`,
   `pallet-htlc/runtime-benchmarks`, `pallet-htlc/try-runtime` to the matching features.
4. `runtime/src/lib.rs` (the chain uses the `#[frame_support::runtime]` macro, not
   `construct_runtime!`). 24 is the next free index:

   ```rust
   #[runtime::pallet_index(24)]
   pub type Htlc = pallet_htlc;
   ```

5. `runtime/src/configs/mod.rs`:

   ```rust
   parameter_types! {
       pub const HtlcMinLock: Balance = 10 * MILLI_UNIT;          // 0.01 QTC (>= ED = 0.001)
       pub const HtlcMinDuration: BlockNumber = 300;               // ~1 h at 12 s
       pub const HtlcMaxDuration: BlockNumber = 7 * DAYS;          // 50_400 blocks
       pub const HtlcMaxLocksPerAccount: u32 = 64;
       pub const HtlcLockDeposit: Balance = scale_fee(10 * MILLI_UNIT); // = multisig ProposalDeposit
   }

   impl pallet_htlc::Config for Runtime {
       type RuntimeHoldReason = RuntimeHoldReason;
       type Currency = Balances;
       type MinLock = HtlcMinLock;
       type MinDuration = HtlcMinDuration;
       type MaxDuration = HtlcMaxDuration;
       type MaxLocksPerAccount = HtlcMaxLocksPerAccount;
       type LockDeposit = HtlcLockDeposit;
       type WeightInfo = pallet_htlc::weights::SubstrateWeight<Runtime>; // TODO: benchmark
   }
   ```

   `Balances` already uses `RuntimeHoldReason`, and its `MaxHolds` follows
   `VariantCountOf<RuntimeHoldReason>`, so the two new hold reasons need no other change.
6. Add `[pallet_htlc, Htlc]` to the runtime's `define_benchmarks!`, run the benchmark CLI and
   overwrite `weights.rs` (the command is in the file header).

### Runtime-specific follow-ups (please review)

- **Wormhole proof recorder.** `WormholeProofRecorderExtension` scans `Balances::TransferOnHold`
  and records a leaf when the destination is a wormhole account, which a claim can trigger.
  `count_transfers` should get `RuntimeCall::Htlc(pallet_htlc::Call::claim { .. }) => 1`, so the
  pre-dispatch weight reserves that leaf insert. Without it, the leaf is recorded but not
  pre-charged.
- **High-security accounts.** The `HighSecurity` whitelist only allows reversible-transfer calls,
  so high-security accounts can't call `Htlc::lock`. That seems like the right default, since a
  lock pays out irreversibly. `claim` and `refund` are permissionless, so any other account can
  settle a high-security account's lock.
- **Weights are placeholders** (`TODO(benchmark)` in `weights.rs`: 50 µs + exact DB r/w counts per
  call). `benchmarking.rs` is provided; generate real weights before enabling.
- `Nonces` entries are never removed (one `u64` per account that ever locked). This is a
  deliberate trade for collision-free ids; drop the nonce from the id if you prefer to prune.

## Tests

`cargo test` runs the unit tests below against the chain's own vendored `pallet_balances` in a mock
runtime. QIP cases 1–8 map one-to-one:

| QIP case | Test |
|---|---|
| 1 lock → claim before expiry, `Claimed` carries preimage | `case1_claim_pays_recipient_and_event_carries_preimage` |
| 2 wrong preimage → error, state unchanged | `case2_wrong_preimage_fails_and_state_unchanged` |
| 3 claim at `now >= expiry` fails; refund works | `case3_claim_at_or_after_expiry_fails_then_refund_works` |
| 4 refund at `now < expiry` fails | `case4_refund_before_expiry_fails` |
| 5 double claim / double refund | `case5_double_settlement_fails` |
| 6 third-party caller | `case6_third_party_claim_and_refund_pay_only_recipient_and_sender` |
| 7 held funds unspendable, `transfer_all` keeps hold | `case7_held_funds_cannot_be_spent_and_transfer_all_keeps_hold` |
| 8 cross-chain vector | `case8_cross_chain_vector_matches_solidity_sha256_bytes32` |

Additional tests cover lock parameter bounds (MinLock, duration bounds inclusive, MaxLocksPerAccount
and slot reuse, ED/insufficient funds), the id formula and nonce, claim creating a non-existent
recipient account, self-lock, claim despite a later freeze, refund after the sender swept its free
balance, independent concurrent locks, bad origins, `integrity_test`, and the benchmark test suite
(when built with `--features runtime-benchmarks`).

### Cross-chain vector

```
S           = 0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20
sha256(S)   = 0xae216c2ef5247a3782c135efa279a3e4cdc61094270f5d2be58c6204b7a612c9
```

`test-vectors/CrossChainVector.t.sol` (Foundry, run inside the `qubi-p2p/contracts` project) checks
`sha256(abi.encodePacked(bytes32 S)) == H`, then locks and **claims a real `QubiHTLC`** with that
`S`. The Rust test checks that the same `S` and `H` claim the Quantus lock, and cross-checks
`sp_io::hashing::sha2_256` against the independent `sha2` crate.

### Results

Run on 2026-09-29, rustc 1.98.1, macOS arm64:

```
$ CARGO_BUILD_JOBS=2 taskpolicy -b cargo test
test result: ok. 23 passed; 0 failed      # 21 pallet tests + 2 mock runtime integrity tests

$ CARGO_BUILD_JOBS=2 taskpolicy -b cargo test --features runtime-benchmarks
test result: ok. 26 passed; 0 failed      # + bench_lock / bench_claim / bench_refund

$ forge test --match-contract CrossChainVector     # in qubi-p2p/contracts
[PASS] test_vector()
  sha256(abi.encodePacked(S)): 0xae216c2ef5247a3782c135efa279a3e4cdc61094270f5d2be58c6204b7a612c9
```

The first cold build (dependencies only) took about 6 minutes with 2 jobs; `target/` is about 1.5 GB.
`cargo fmt` / `clippy` were not run because the components aren't installed locally, and the
`no_std`/WASM build was not run either. Run them in the chain's CI after merging.

## Build notes

- Standalone build: the deps are pinned to the chain commit via git, the chain's
  `[patch.crates-io]` for the vendored FRAME crates is mirrored, and `Cargo.lock` is seeded from the
  chain.
- Toolchain: the chain pins `1.93.0`. This crate was built and tested with the locally installed
  `rustc 1.98.1`, with no toolchain-specific code.
- Builds were run with `CARGO_BUILD_JOBS=2 taskpolicy -b cargo test` to keep the laptop
  responsive. Only this crate was built, not the node or the WASM runtime.

## License

MIT — see `LICENSE`.
