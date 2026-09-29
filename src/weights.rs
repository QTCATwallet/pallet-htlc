//! Weights for `pallet_htlc`.
//!
//! TODO(benchmark): these are hand-written PLACEHOLDERS, not measured values. Before enabling
//! the pallet on a live network, regenerate this file with the chain's benchmark CLI, e.g.
//!
//! ```text
//! ./target/release/quantus-node benchmark pallet --pallet=pallet_htlc --extrinsic=* \
//!   --steps=20 --repeat=50 --wasm-execution=compiled --heap-pages=4096 \
//!   --runtime=./target/release/wbuild/quantus-runtime/quantus_runtime.wasm \
//!   --genesis-builder=runtime --template=./.maintain/frame-weight-template.hbs \
//!   --output=./pallets/htlc/src/weights.rs
//! ```
//!
//! The placeholders are deliberately conservative: a fixed 50 µs of compute per call plus the
//! exact storage accesses each extrinsic performs, costed with `RocksDbWeight`, and a proof size
//! that covers every touched key.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for `pallet_htlc`.
pub trait WeightInfo {
	fn lock() -> Weight;
	fn claim() -> Weight;
	fn refund() -> Weight;
}

/// Placeholder weights for the Quantus runtime. TODO(benchmark): replace with generated weights.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// Storage: `Htlc::LockCount` (r:1 w:1), `Htlc::Nonces` (r:1 w:1), `Htlc::Locks` (r:1 w:1),
	/// `Balances::Holds` (r:1 w:1) x2, `System::Account` (r:1 w:1)
	fn lock() -> Weight {
		Weight::from_parts(50_000_000, 6_000)
			.saturating_add(T::DbWeight::get().reads(5_u64))
			.saturating_add(T::DbWeight::get().writes(6_u64))
	}
	/// Storage: `Htlc::Locks` (r:1 w:1), `Htlc::LockCount` (r:1 w:1), `Balances::Holds` (r:1 w:1),
	/// `System::Account` (r:2 w:2)
	fn claim() -> Weight {
		Weight::from_parts(50_000_000, 8_000)
			.saturating_add(T::DbWeight::get().reads(5_u64))
			.saturating_add(T::DbWeight::get().writes(5_u64))
	}
	/// Storage: `Htlc::Locks` (r:1 w:1), `Htlc::LockCount` (r:1 w:1), `Balances::Holds` (r:1 w:1),
	/// `System::Account` (r:1 w:1)
	fn refund() -> Weight {
		Weight::from_parts(50_000_000, 6_000)
			.saturating_add(T::DbWeight::get().reads(4_u64))
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}
}

// For backwards compatibility and tests.
impl WeightInfo for () {
	fn lock() -> Weight {
		Weight::from_parts(50_000_000, 6_000)
			.saturating_add(RocksDbWeight::get().reads(5_u64))
			.saturating_add(RocksDbWeight::get().writes(6_u64))
	}
	fn claim() -> Weight {
		Weight::from_parts(50_000_000, 8_000)
			.saturating_add(RocksDbWeight::get().reads(5_u64))
			.saturating_add(RocksDbWeight::get().writes(5_u64))
	}
	fn refund() -> Weight {
		Weight::from_parts(50_000_000, 6_000)
			.saturating_add(RocksDbWeight::get().reads(4_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}
}
