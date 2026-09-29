//! # HTLC Pallet
//!
//! Native hash-time-locked transfers for Quantus, so QTC can be swapped trustlessly against any
//! chain that already has HTLCs (every EVM chain, TRON, Bitcoin) — no bots, no custodian, no
//! admin key.
//!
//! ## Overview
//!
//! - [`Pallet::lock`]: the sender places a hold on `amount` of its own free balance for
//!   `recipient`, under `hashlock = sha256(S)` for a 32-byte secret `S`, until block `expiry`.
//! - [`Pallet::claim`]: **anyone** may reveal `S` before `expiry`; the held funds move to
//!   `recipient` and `S` is published in [`Event::Claimed`] so the counterparty chain can be
//!   settled with it.
//! - [`Pallet::refund`]: **anyone** may call it at or after `expiry`; the hold is released back to
//!   the sender.
//!
//! Funds can only ever end up with `recipient` (claim) or `sender` (refund), so a third-party
//! caller gains nothing. There is no admin path, no amend, no early cancel and no partial claim:
//! anything that lets the sender pull funds before `expiry` would break the swap.
//!
//! ## Cross-chain compatibility
//!
//! The preimage is exactly 32 bytes and the hash is plain SHA-256 over those 32 bytes, i.e. the
//! same value as Solidity's `sha256(abi.encodePacked(bytes32 preimage))`. Fixing the length on both
//! sides removes the "accepted on one chain, rejected on the other" length-mismatch attack.
//!
//! ## Funds accounting
//!
//! Funds never leave the sender's account until claimed: the principal is held under
//! [`HoldReason::HtlcLock`] and the storage deposit under [`HoldReason::HtlcDeposit`]. There is no
//! pooled pallet account. A claim moves the principal with `transfer_on_hold` (emitting
//! `Balances::TransferOnHold`), a refund `release`s it.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod weights;
pub use weights::*;

use codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
use sp_runtime::RuntimeDebug;
use scale_info::TypeInfo;

/// Identifier of a lock: `blake2_256(SCALE(sender, recipient, amount, hashlock, expiry, nonce))`.
pub type LockId = sp_core::H256;

/// SHA-256 digest of the 32-byte preimage.
pub type HashLock = [u8; 32];

/// The 32-byte secret whose SHA-256 is the hashlock.
pub type Preimage = [u8; 32];

/// A single open hash-time lock.
#[derive(
	Encode,
	Decode,
	DecodeWithMemTracking,
	MaxEncodedLen,
	Clone,
	TypeInfo,
	RuntimeDebug,
	PartialEq,
	Eq,
)]
pub struct HtlcLock<AccountId, Balance, BlockNumber> {
	/// Account whose funds are held and which receives them back on refund.
	pub sender: AccountId,
	/// Account which receives the funds on claim.
	pub recipient: AccountId,
	/// Principal held under [`HoldReason::HtlcLock`].
	pub amount: Balance,
	/// `sha256(preimage)`.
	pub hashlock: HashLock,
	/// First block at which the lock can no longer be claimed and can be refunded.
	pub expiry: BlockNumber,
	/// Storage deposit held under [`HoldReason::HtlcDeposit`]; always returned to `sender`.
	pub deposit: Balance,
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::{
		pallet_prelude::*,
		traits::{
			fungible::{Inspect, InspectHold, Mutate, MutateHold},
			tokens::{Fortitude, Precision, Restriction},
		},
	};
	use frame_system::pallet_prelude::*;
	use sp_runtime::traits::{Saturating, Zero};

	/// Balance type of the configured currency.
	pub type BalanceOf<T> =
		<<T as Config>::Currency as Inspect<<T as frame_system::Config>::AccountId>>::Balance;

	/// Lock record type for this runtime.
	pub type HtlcLockOf<T> =
		HtlcLock<<T as frame_system::Config>::AccountId, BalanceOf<T>, BlockNumberFor<T>>;

	/// The in-code storage version.
	const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config<RuntimeEvent: From<Event<Self>>> {
		/// The overarching hold reason.
		type RuntimeHoldReason: From<HoldReason>;

		/// Native currency supporting fungible holds (`pallet_balances` in the runtime).
		type Currency: Inspect<Self::AccountId>
			+ Mutate<Self::AccountId>
			+ InspectHold<Self::AccountId, Reason = Self::RuntimeHoldReason>
			+ MutateHold<Self::AccountId, Reason = Self::RuntimeHoldReason>;

		/// Smallest principal that can be locked. Must be at least the existential deposit, so
		/// that a claim can always create the recipient's account.
		#[pallet::constant]
		type MinLock: Get<BalanceOf<Self>>;

		/// Minimum `expiry - now` at lock time, in blocks.
		#[pallet::constant]
		type MinDuration: Get<BlockNumberFor<Self>>;

		/// Maximum `expiry - now` at lock time, in blocks.
		#[pallet::constant]
		type MaxDuration: Get<BlockNumberFor<Self>>;

		/// Maximum number of open locks a single sender may have.
		#[pallet::constant]
		type MaxLocksPerAccount: Get<u32>;

		/// Storage deposit held from the sender per open lock (returned on claim and refund).
		#[pallet::constant]
		type LockDeposit: Get<BalanceOf<Self>>;

		/// Weight information for extrinsics.
		type WeightInfo: WeightInfo;
	}

	/// A reason for the pallet placing a hold on funds.
	#[pallet::composite_enum]
	pub enum HoldReason {
		/// Principal of an open HTLC.
		#[codec(index = 0)]
		HtlcLock,
		/// Storage deposit of an open HTLC.
		#[codec(index = 1)]
		HtlcDeposit,
	}

	/// Open locks by id.
	#[pallet::storage]
	pub type Locks<T: Config> = StorageMap<_, Identity, LockId, HtlcLockOf<T>, OptionQuery>;

	/// Number of open locks per sender (bounded by `MaxLocksPerAccount`).
	#[pallet::storage]
	pub type LockCount<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

	/// Per-sender monotonically increasing nonce mixed into the lock id, so identical parameters
	/// never produce the same id twice.
	#[pallet::storage]
	pub type Nonces<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u64, ValueQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Funds were locked.
		Locked {
			id: LockId,
			sender: T::AccountId,
			recipient: T::AccountId,
			amount: BalanceOf<T>,
			hashlock: HashLock,
			expiry: BlockNumberFor<T>,
		},
		/// Funds were paid to the recipient. `preimage` is the secret that settles the other
		/// chain.
		Claimed { id: LockId, preimage: Preimage },
		/// Funds were returned to the sender after expiry.
		Refunded { id: LockId },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// `amount` is below `MinLock`.
		AmountTooLow,
		/// `expiry` is earlier than `now + MinDuration`.
		ExpiryTooSoon,
		/// `expiry` is later than `now + MaxDuration`.
		ExpiryTooLate,
		/// The sender already has `MaxLocksPerAccount` open locks.
		TooManyLocks,
		/// A lock with this id already exists.
		LockExists,
		/// No open lock with this id (never existed, or already claimed / refunded).
		LockNotFound,
		/// `sha256(preimage) != hashlock`.
		InvalidPreimage,
		/// The lock has expired and can no longer be claimed.
		LockExpired,
		/// The lock has not expired yet and cannot be refunded.
		LockNotExpired,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn integrity_test() {
			assert!(
				T::MinLock::get() >= T::Currency::minimum_balance(),
				"MinLock must be >= the existential deposit, otherwise a claim to a fresh \
				 recipient account could fail"
			);
			assert!(!T::MinLock::get().is_zero(), "MinLock must be non-zero");
			assert!(!T::MinDuration::get().is_zero(), "MinDuration must be non-zero");
			assert!(
				T::MinDuration::get() <= T::MaxDuration::get(),
				"MinDuration must be <= MaxDuration"
			);
			assert!(T::MaxLocksPerAccount::get() > 0, "MaxLocksPerAccount must be non-zero");
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Lock `amount` of the caller's free balance for `recipient` under `hashlock` until block
		/// `expiry`.
		///
		/// Places a hold (`HoldReason::HtlcLock`) on `amount` and a second hold
		/// (`HoldReason::HtlcDeposit`) on `LockDeposit`. The caller must keep at least the
		/// existential deposit free.
		#[pallet::call_index(0)]
		#[pallet::weight(T::WeightInfo::lock())]
		pub fn lock(
			origin: OriginFor<T>,
			recipient: T::AccountId,
			#[pallet::compact] amount: BalanceOf<T>,
			hashlock: HashLock,
			expiry: BlockNumberFor<T>,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			ensure!(amount >= T::MinLock::get(), Error::<T>::AmountTooLow);

			let now = frame_system::Pallet::<T>::block_number();
			ensure!(expiry >= now.saturating_add(T::MinDuration::get()), Error::<T>::ExpiryTooSoon);
			ensure!(expiry <= now.saturating_add(T::MaxDuration::get()), Error::<T>::ExpiryTooLate);

			let open = LockCount::<T>::get(&sender);
			ensure!(open < T::MaxLocksPerAccount::get(), Error::<T>::TooManyLocks);

			let nonce = Nonces::<T>::get(&sender);
			let id = Self::lock_id(&sender, &recipient, amount, &hashlock, expiry, nonce);
			ensure!(!Locks::<T>::contains_key(id), Error::<T>::LockExists);

			let deposit = T::LockDeposit::get();
			// `hold` fails atomically (and the extrinsic is transactional) if the caller cannot
			// cover principal + deposit while keeping the existential deposit free.
			T::Currency::hold(&HoldReason::HtlcLock.into(), &sender, amount)?;
			if !deposit.is_zero() {
				T::Currency::hold(&HoldReason::HtlcDeposit.into(), &sender, deposit)?;
			}

			Locks::<T>::insert(
				id,
				HtlcLock {
					sender: sender.clone(),
					recipient: recipient.clone(),
					amount,
					hashlock,
					expiry,
					deposit,
				},
			);
			LockCount::<T>::insert(&sender, open.saturating_add(1));
			Nonces::<T>::insert(&sender, nonce.wrapping_add(1));

			Self::deposit_event(Event::Locked { id, sender, recipient, amount, hashlock, expiry });
			Ok(())
		}

		/// Pay an open lock to its recipient by revealing the preimage. Callable by anyone.
		///
		/// Requires `now < expiry` and `sha256(preimage) == hashlock`.
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::claim())]
		pub fn claim(origin: OriginFor<T>, id: LockId, preimage: Preimage) -> DispatchResult {
			ensure_signed(origin)?;

			let lock = Locks::<T>::get(id).ok_or(Error::<T>::LockNotFound)?;
			let now = frame_system::Pallet::<T>::block_number();
			ensure!(now < lock.expiry, Error::<T>::LockExpired);
			ensure!(sp_io::hashing::sha2_256(&preimage) == lock.hashlock, Error::<T>::InvalidPreimage);

			Self::remove_lock(id, &lock);

			if lock.recipient == lock.sender {
				// Hold -> free on the same account is not a credit; don't emit a self-directed
				// `TransferOnHold` (mirrors `pallet_reversible_transfers`).
				Self::release_principal(&lock);
			} else {
				// `Fortitude::Force`: the principal was committed at lock time, so a freeze added
				// or balance lock placed on the sender *afterwards* (by any current or future
				// pallet) must not be able to block the claim — otherwise the sender could
				// collect the counter-asset with the secret and still refund the QTC after
				// expiry.
				//
				// `Precision::BestEffort` + `MinLock >= ED` (checked in `integrity_test`) mean this
				// cannot fail for a live lock; the error branch is purely defensive.
				let moved = T::Currency::transfer_on_hold(
					&HoldReason::HtlcLock.into(),
					&lock.sender,
					&lock.recipient,
					lock.amount,
					Precision::BestEffort,
					Restriction::Free,
					Fortitude::Force,
				)?;
				if moved != lock.amount {
					log::error!(
						target: "runtime::htlc",
						"claim {:?}: moved {:?} of {:?} held; hold was reduced externally",
						id, moved, lock.amount,
					);
				}
			}
			Self::release_deposit(&lock);

			Self::deposit_event(Event::Claimed { id, preimage });
			Ok(())
		}

		/// Return an expired lock to its sender. Callable by anyone.
		///
		/// Requires `now >= expiry`.
		#[pallet::call_index(2)]
		#[pallet::weight(T::WeightInfo::refund())]
		pub fn refund(origin: OriginFor<T>, id: LockId) -> DispatchResult {
			ensure_signed(origin)?;

			let lock = Locks::<T>::get(id).ok_or(Error::<T>::LockNotFound)?;
			let now = frame_system::Pallet::<T>::block_number();
			ensure!(now >= lock.expiry, Error::<T>::LockNotExpired);

			Self::remove_lock(id, &lock);
			Self::release_principal(&lock);
			Self::release_deposit(&lock);

			Self::deposit_event(Event::Refunded { id });
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// `blake2_256(SCALE(sender, recipient, amount, hashlock, expiry, nonce))`.
		///
		/// Off-chain software can recompute the id before submitting `lock` (the sender's nonce is
		/// readable from [`Nonces`]), or simply read it from [`Event::Locked`].
		pub fn lock_id(
			sender: &T::AccountId,
			recipient: &T::AccountId,
			amount: BalanceOf<T>,
			hashlock: &HashLock,
			expiry: BlockNumberFor<T>,
			nonce: u64,
		) -> LockId {
			(sender, recipient, amount, hashlock, expiry, nonce)
				.using_encoded(sp_io::hashing::blake2_256)
				.into()
		}

		fn remove_lock(id: LockId, lock: &HtlcLockOf<T>) {
			Locks::<T>::remove(id);
			LockCount::<T>::mutate_exists(&lock.sender, |count| {
				let next = count.unwrap_or(0).saturating_sub(1);
				*count = if next == 0 { None } else { Some(next) };
			});
		}

		/// Release the principal back to the sender's free balance.
		///
		/// `release` does not check freezes and never kills the account (the account keeps
		/// existing: the funds only move from reserved to free), so with `BestEffort` it cannot
		/// fail for a live lock. A failure is logged, never propagated, so the lock can't be
		/// stuck.
		fn release_principal(lock: &HtlcLockOf<T>) {
			Self::release_or_log(&HoldReason::HtlcLock.into(), &lock.sender, lock.amount);
		}

		fn release_deposit(lock: &HtlcLockOf<T>) {
			if !lock.deposit.is_zero() {
				Self::release_or_log(&HoldReason::HtlcDeposit.into(), &lock.sender, lock.deposit);
			}
		}

		fn release_or_log(reason: &T::RuntimeHoldReason, who: &T::AccountId, amount: BalanceOf<T>) {
			match T::Currency::release(reason, who, amount, Precision::BestEffort) {
				Ok(released) if released == amount => {},
				Ok(released) => log::error!(
					target: "runtime::htlc",
					"released {:?} of {:?} for {:?}; hold was reduced externally",
					released, amount, who,
				),
				Err(e) => log::error!(
					target: "runtime::htlc",
					"release of {:?} for {:?} failed: {:?}",
					amount, who, e,
				),
			}
		}
	}
}
