//! Benchmarks for `pallet_htlc`. Each extrinsic has a single, constant-cost path.

use super::*;
use crate::Pallet as Htlc;
use frame_benchmarking::v2::*;
use frame_support::traits::{
	fungible::{Inspect, Mutate},
	Get,
};
use frame_system::{pallet_prelude::BlockNumberFor, RawOrigin};
use sp_runtime::traits::Saturating;

const SEED: u32 = 0;
/// Benchmark preimage; its SHA-256 is computed at setup.
const PREIMAGE: Preimage = [7u8; 32];

fn funded<T: Config>(name: &'static str) -> T::AccountId {
	let who: T::AccountId = account(name, 0, SEED);
	let amount = T::MinLock::get()
		.saturating_add(T::LockDeposit::get())
		.saturating_mul(4u32.into())
		.saturating_add(T::Currency::minimum_balance().saturating_mul(4u32.into()));
	T::Currency::set_balance(&who, amount);
	who
}

fn open_lock<T: Config>() -> (LockId, T::AccountId, BlockNumberFor<T>) {
	let sender = funded::<T>("sender");
	let recipient: T::AccountId = account("recipient", 0, SEED);
	let now = frame_system::Pallet::<T>::block_number();
	let expiry = now.saturating_add(T::MinDuration::get());
	let hashlock = sp_io::hashing::sha2_256(&PREIMAGE);
	let nonce = Nonces::<T>::get(&sender);
	let id = Htlc::<T>::lock_id(&sender, &recipient, T::MinLock::get(), &hashlock, expiry, nonce);
	Htlc::<T>::lock(
		RawOrigin::Signed(sender.clone()).into(),
		recipient,
		T::MinLock::get(),
		hashlock,
		expiry,
	)
	.expect("lock must succeed in benchmark setup");
	(id, sender, expiry)
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn lock() {
		let sender = funded::<T>("sender");
		let recipient: T::AccountId = account("recipient", 0, SEED);
		let expiry =
			frame_system::Pallet::<T>::block_number().saturating_add(T::MaxDuration::get());
		let hashlock = sp_io::hashing::sha2_256(&PREIMAGE);

		#[extrinsic_call]
		_(RawOrigin::Signed(sender.clone()), recipient, T::MinLock::get(), hashlock, expiry);

		assert_eq!(LockCount::<T>::get(&sender), 1);
	}

	#[benchmark]
	fn claim() {
		let (id, _, _) = open_lock::<T>();
		// Worst case: recipient account does not exist yet and gets created.
		let caller = funded::<T>("relayer");

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), id, PREIMAGE);

		assert!(!Locks::<T>::contains_key(id));
	}

	#[benchmark]
	fn refund() {
		let (id, _, expiry) = open_lock::<T>();
		frame_system::Pallet::<T>::set_block_number(expiry);
		let caller = funded::<T>("relayer");

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), id);

		assert!(!Locks::<T>::contains_key(id));
	}

	impl_benchmark_test_suite!(Htlc, crate::mock::new_test_ext(), crate::mock::Test);
}
