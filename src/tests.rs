//! Unit tests. Cases 1–8 map one-to-one to "Test cases we'd run against it" in QIP_HTLC.md.

use crate::{mock::*, Error, Event, HoldReason, Locks, LockCount, LockId, Nonces};
use frame_support::{
	assert_noop, assert_ok,
	traits::{
		fungible::{Inspect, InspectHold, MutateFreeze},
		tokens::{Fortitude, Preservation},
	},
};
use hex_literal::hex;
use sha2::{Digest, Sha256};
use sp_runtime::{DispatchError, TokenError};

/// Fixed cross-chain secret `S` (bytes 0x01..=0x20).
const S: [u8; 32] = hex!("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
/// `sha256(abi.encodePacked(bytes32 S))`, computed independently with Foundry against
/// `QubiHTLC.sol` (see README "Cross-chain vector"). Also equals `sha256` of the raw 32 bytes.
const H: [u8; 32] = hex!("ae216c2ef5247a3782c135efa279a3e4cdc61094270f5d2be58c6204b7a612c9");

const AMOUNT: Balance = 1_000;

fn free(who: AccountId) -> Balance {
	Balances::free_balance(who)
}

fn held(reason: HoldReason, who: AccountId) -> Balance {
	<Balances as InspectHold<_>>::balance_on_hold(&reason.into(), &who)
}

fn now() -> u64 {
	System::block_number()
}

fn last_event() -> RuntimeEvent {
	System::events().pop().expect("an event").event
}

/// Lock `AMOUNT` from ALICE to BOB under `H`, expiring `MIN_DURATION * 2` blocks from now.
fn lock_default() -> (LockId, u64) {
	let expiry = now() + MIN_DURATION * 2;
	lock_with(ALICE, BOB, AMOUNT, expiry)
}

fn lock_with(sender: AccountId, recipient: AccountId, amount: Balance, expiry: u64) -> (LockId, u64) {
	let nonce = Nonces::<Test>::get(sender);
	let id = Htlc::lock_id(&sender, &recipient, amount, &H, expiry, nonce);
	assert_ok!(Htlc::lock(RuntimeOrigin::signed(sender), recipient, amount, H, expiry));
	(id, expiry)
}

// --------------------------------------------------------------------------------------------
// lock
// --------------------------------------------------------------------------------------------

#[test]
fn lock_places_holds_and_emits_locked() {
	new_test_ext().execute_with(|| {
		let (id, expiry) = lock_default();

		assert_eq!(free(ALICE), 10_000 - AMOUNT - DEPOSIT);
		assert_eq!(held(HoldReason::HtlcLock, ALICE), AMOUNT);
		assert_eq!(held(HoldReason::HtlcDeposit, ALICE), DEPOSIT);
		// Funds never leave the sender's account while locked.
		assert_eq!(Balances::total_balance(&ALICE), 10_000);
		assert_eq!(free(BOB), 1_000);

		let lock = Locks::<Test>::get(id).expect("stored");
		assert_eq!(
			(lock.sender, lock.recipient, lock.amount, lock.hashlock, lock.expiry, lock.deposit),
			(ALICE, BOB, AMOUNT, H, expiry, DEPOSIT)
		);
		assert_eq!(LockCount::<Test>::get(ALICE), 1);
		assert_eq!(Nonces::<Test>::get(ALICE), 1);
		assert_eq!(
			last_event(),
			RuntimeEvent::Htlc(Event::Locked {
				id,
				sender: ALICE,
				recipient: BOB,
				amount: AMOUNT,
				hashlock: H,
				expiry,
			})
		);
	});
}

#[test]
fn lock_id_is_blake2_of_scale_params_and_nonce() {
	new_test_ext().execute_with(|| {
		use codec::Encode;
		let expiry = now() + MIN_DURATION;
		let expected: LockId =
			sp_io::hashing::blake2_256(&(ALICE, BOB, AMOUNT, H, expiry, 0u64).encode()).into();
		let (id0, _) = lock_with(ALICE, BOB, AMOUNT, expiry);
		assert_eq!(id0, expected);
		// Identical parameters give a fresh id thanks to the per-account nonce.
		let (id1, _) = lock_with(ALICE, BOB, AMOUNT, expiry);
		assert_ne!(id0, id1);
		assert!(Locks::<Test>::contains_key(id0) && Locks::<Test>::contains_key(id1));
	});
}

#[test]
fn lock_enforces_min_lock() {
	new_test_ext().execute_with(|| {
		let expiry = now() + MIN_DURATION;
		assert_noop!(
			Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, MIN_LOCK - 1, H, expiry),
			Error::<Test>::AmountTooLow
		);
		assert_ok!(Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, MIN_LOCK, H, expiry));
	});
}

#[test]
fn lock_enforces_duration_bounds() {
	new_test_ext().execute_with(|| {
		System::set_block_number(50);
		assert_noop!(
			Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, AMOUNT, H, 50 + MIN_DURATION - 1),
			Error::<Test>::ExpiryTooSoon
		);
		assert_noop!(
			Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, AMOUNT, H, 49),
			Error::<Test>::ExpiryTooSoon
		);
		assert_noop!(
			Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, AMOUNT, H, 50 + MAX_DURATION + 1),
			Error::<Test>::ExpiryTooLate
		);
		// Both bounds are inclusive.
		assert_ok!(Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, AMOUNT, H, 50 + MIN_DURATION));
		assert_ok!(Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, AMOUNT, H, 50 + MAX_DURATION));
	});
}

#[test]
fn lock_enforces_max_locks_per_account_and_frees_slots() {
	new_test_ext().execute_with(|| {
		let mut ids = vec![];
		for _ in 0..MAX_LOCKS {
			ids.push(lock_default().0);
		}
		let expiry = now() + MIN_DURATION;
		assert_noop!(
			Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, AMOUNT, H, expiry),
			Error::<Test>::TooManyLocks
		);
		// Another sender is unaffected.
		assert_ok!(Htlc::lock(RuntimeOrigin::signed(BOB), ALICE, MIN_LOCK, H, expiry));
		// Settling a lock frees a slot.
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(CHARLIE), ids[0], S));
		assert_eq!(LockCount::<Test>::get(ALICE), MAX_LOCKS - 1);
		assert_ok!(Htlc::lock(RuntimeOrigin::signed(ALICE), BOB, AMOUNT, H, expiry));
	});
}

#[test]
fn lock_requires_funds_for_amount_deposit_and_ed() {
	new_test_ext().execute_with(|| {
		// BOB has 1_000: locking 1_000 - DEPOSIT would leave 0 free (< ED).
		let expiry = now() + MIN_DURATION;
		assert!(Htlc::lock(RuntimeOrigin::signed(BOB), ALICE, 1_000 - DEPOSIT, H, expiry).is_err());
		assert_eq!(held(HoldReason::HtlcLock, BOB), 0);
		assert_eq!(held(HoldReason::HtlcDeposit, BOB), 0);
		assert_eq!(LockCount::<Test>::get(BOB), 0);
		// Leaving exactly ED free works.
		assert_ok!(Htlc::lock(RuntimeOrigin::signed(BOB), ALICE, 1_000 - DEPOSIT - ED, H, expiry));
		assert_eq!(free(BOB), ED);
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 1: lock -> claim with correct preimage before expiry
// --------------------------------------------------------------------------------------------

#[test]
fn case1_claim_pays_recipient_and_event_carries_preimage() {
	new_test_ext().execute_with(|| {
		let (id, expiry) = lock_default();
		System::set_block_number(expiry - 1); // last claimable block

		assert_ok!(Htlc::claim(RuntimeOrigin::signed(BOB), id, S));

		assert_eq!(free(BOB), 1_000 + AMOUNT);
		assert_eq!(free(ALICE), 10_000 - AMOUNT); // deposit returned
		assert_eq!(held(HoldReason::HtlcLock, ALICE), 0);
		assert_eq!(held(HoldReason::HtlcDeposit, ALICE), 0);
		assert!(!Locks::<Test>::contains_key(id));
		assert_eq!(LockCount::<Test>::get(ALICE), 0);
		assert_eq!(last_event(), RuntimeEvent::Htlc(Event::Claimed { id, preimage: S }));
		// Value moved with `transfer_on_hold` (what the chain's wormhole event scanner records).
		assert!(System::events().iter().any(|r| matches!(
			r.event,
			RuntimeEvent::Balances(pallet_balances::Event::TransferOnHold {
				source: ALICE, dest: BOB, amount: AMOUNT, ..
			})
		)));
	});
}

#[test]
fn claim_creates_nonexistent_recipient_account() {
	new_test_ext().execute_with(|| {
		assert_eq!(Balances::total_balance(&DAVE), 0);
		let (id, _) = lock_with(ALICE, DAVE, MIN_LOCK, now() + MIN_DURATION);
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(CHARLIE), id, S));
		assert_eq!(free(DAVE), MIN_LOCK);
		assert!(System::account_exists(&DAVE));
	});
}

#[test]
fn claim_to_self_releases_without_transfer_event() {
	new_test_ext().execute_with(|| {
		let (id, _) = lock_with(ALICE, ALICE, AMOUNT, now() + MIN_DURATION);
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(CHARLIE), id, S));
		assert_eq!(free(ALICE), 10_000);
		assert!(!System::events().iter().any(|r| matches!(
			r.event,
			RuntimeEvent::Balances(pallet_balances::Event::TransferOnHold { .. })
		)));
	});
}

#[test]
fn claim_cannot_be_blocked_by_a_later_freeze_on_sender() {
	new_test_ext().execute_with(|| {
		let (id, _) = lock_default();
		// After locking, the sender gets its whole balance frozen by some other pallet (Quantus
		// has no freeze users today; this guards future ones). A `Fortitude::Polite` claim would fail here and let the sender refund after
		// expiry while already holding the counter-asset.
		assert_ok!(<Balances as MutateFreeze<_>>::set_freeze(&*b"otherpal", &ALICE, 10_000));
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(CHARLIE), id, S));
		assert_eq!(free(BOB), 1_000 + AMOUNT);
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 2: wrong preimage
// --------------------------------------------------------------------------------------------

#[test]
fn case2_wrong_preimage_fails_and_state_unchanged() {
	new_test_ext().execute_with(|| {
		let (id, _) = lock_default();
		let mut wrong = S;
		wrong[31] ^= 1;
		assert_noop!(Htlc::claim(RuntimeOrigin::signed(BOB), id, wrong), Error::<Test>::InvalidPreimage);
		// The hashlock itself is not a valid preimage either.
		assert_noop!(Htlc::claim(RuntimeOrigin::signed(BOB), id, H), Error::<Test>::InvalidPreimage);
		assert!(Locks::<Test>::contains_key(id));
		assert_eq!(held(HoldReason::HtlcLock, ALICE), AMOUNT);
		assert_eq!(free(BOB), 1_000);
		// Still claimable with the right one.
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(BOB), id, S));
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 3: claim at now >= expiry fails; refund works
// --------------------------------------------------------------------------------------------

#[test]
fn case3_claim_at_or_after_expiry_fails_then_refund_works() {
	new_test_ext().execute_with(|| {
		let (id, expiry) = lock_default();

		System::set_block_number(expiry);
		assert_noop!(Htlc::claim(RuntimeOrigin::signed(BOB), id, S), Error::<Test>::LockExpired);
		System::set_block_number(expiry + 100);
		assert_noop!(Htlc::claim(RuntimeOrigin::signed(BOB), id, S), Error::<Test>::LockExpired);

		assert_ok!(Htlc::refund(RuntimeOrigin::signed(ALICE), id));
		assert_eq!(free(ALICE), 10_000);
		assert_eq!(held(HoldReason::HtlcLock, ALICE), 0);
		assert_eq!(held(HoldReason::HtlcDeposit, ALICE), 0);
		assert_eq!(free(BOB), 1_000);
		assert!(!Locks::<Test>::contains_key(id));
		assert_eq!(LockCount::<Test>::get(ALICE), 0);
		assert_eq!(last_event(), RuntimeEvent::Htlc(Event::Refunded { id }));
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 4: refund before expiry fails
// --------------------------------------------------------------------------------------------

#[test]
fn case4_refund_before_expiry_fails() {
	new_test_ext().execute_with(|| {
		let (id, expiry) = lock_default();
		assert_noop!(Htlc::refund(RuntimeOrigin::signed(ALICE), id), Error::<Test>::LockNotExpired);
		System::set_block_number(expiry - 1);
		assert_noop!(Htlc::refund(RuntimeOrigin::signed(ALICE), id), Error::<Test>::LockNotExpired);
		assert_eq!(held(HoldReason::HtlcLock, ALICE), AMOUNT);
		// Boundary: refundable exactly at `expiry` (claim and refund windows never overlap).
		System::set_block_number(expiry);
		assert_ok!(Htlc::refund(RuntimeOrigin::signed(ALICE), id));
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 5: double claim / double refund / claim-after-refund
// --------------------------------------------------------------------------------------------

#[test]
fn case5_double_settlement_fails() {
	new_test_ext().execute_with(|| {
		// double claim
		let (id, expiry) = lock_default();
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(BOB), id, S));
		assert_noop!(Htlc::claim(RuntimeOrigin::signed(BOB), id, S), Error::<Test>::LockNotFound);
		// refund after claim
		System::set_block_number(expiry);
		assert_noop!(Htlc::refund(RuntimeOrigin::signed(ALICE), id), Error::<Test>::LockNotFound);
		assert_eq!(free(BOB), 1_000 + AMOUNT);

		// double refund
		let (id2, expiry2) = lock_default();
		System::set_block_number(expiry2);
		assert_ok!(Htlc::refund(RuntimeOrigin::signed(ALICE), id2));
		assert_noop!(Htlc::refund(RuntimeOrigin::signed(ALICE), id2), Error::<Test>::LockNotFound);
		assert_eq!(free(ALICE), 10_000 - AMOUNT);

		// unknown id
		assert_noop!(
			Htlc::claim(RuntimeOrigin::signed(BOB), LockId::repeat_byte(9), S),
			Error::<Test>::LockNotFound
		);
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 6: third-party caller
// --------------------------------------------------------------------------------------------

#[test]
fn case6_third_party_claim_and_refund_pay_only_recipient_and_sender() {
	new_test_ext().execute_with(|| {
		let (id, _) = lock_default();
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(CHARLIE), id, S));
		assert_eq!(free(BOB), 1_000 + AMOUNT);
		assert_eq!(free(ALICE), 10_000 - AMOUNT);
		assert_eq!(free(CHARLIE), 1_000);

		let (id2, expiry2) = lock_default();
		System::set_block_number(expiry2);
		assert_ok!(Htlc::refund(RuntimeOrigin::signed(CHARLIE), id2));
		assert_eq!(free(ALICE), 10_000 - AMOUNT);
		assert_eq!(free(BOB), 1_000 + AMOUNT);
		assert_eq!(free(CHARLIE), 1_000);
	});
}

#[test]
fn unsigned_origins_are_rejected() {
	new_test_ext().execute_with(|| {
		let (id, _) = lock_default();
		assert_noop!(Htlc::claim(RuntimeOrigin::none(), id, S), DispatchError::BadOrigin);
		assert_noop!(Htlc::refund(RuntimeOrigin::root(), id), DispatchError::BadOrigin);
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 7: held funds are unspendable; transfer_all leaves the hold intact
// --------------------------------------------------------------------------------------------

#[test]
fn case7_held_funds_cannot_be_spent_and_transfer_all_keeps_hold() {
	new_test_ext().execute_with(|| {
		let (id, _) = lock_default();
		// With holds present the account can't be reaped, so ED stays locked in free balance.
		let spendable = 10_000 - AMOUNT - DEPOSIT - ED;
		assert_eq!(
			Balances::reducible_balance(&ALICE, Preservation::Expendable, Fortitude::Polite),
			spendable
		);

		// Cannot dip into held funds (or the ED the holds pin in place).
		assert_noop!(
			Balances::transfer_allow_death(RuntimeOrigin::signed(ALICE), CHARLIE, spendable + 1),
			TokenError::Frozen
		);

		// transfer_all sweeps the free balance but not the holds.
		assert_ok!(Balances::transfer_all(RuntimeOrigin::signed(ALICE), CHARLIE, false));
		assert_eq!(held(HoldReason::HtlcLock, ALICE), AMOUNT);
		assert_eq!(held(HoldReason::HtlcDeposit, ALICE), DEPOSIT);
		assert!(System::account_exists(&ALICE));

		// The lock still settles normally afterwards.
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(CHARLIE), id, S));
		assert_eq!(free(BOB), 1_000 + AMOUNT);
		assert_eq!(held(HoldReason::HtlcLock, ALICE), 0);
		assert_eq!(held(HoldReason::HtlcDeposit, ALICE), 0);
	});
}

#[test]
fn refund_works_after_sender_swept_free_balance() {
	new_test_ext().execute_with(|| {
		let (id, expiry) = lock_default();
		assert_ok!(Balances::transfer_all(RuntimeOrigin::signed(ALICE), CHARLIE, false));
		let left = free(ALICE);
		System::set_block_number(expiry);
		assert_ok!(Htlc::refund(RuntimeOrigin::signed(CHARLIE), id));
		assert_eq!(free(ALICE), left + AMOUNT + DEPOSIT);
	});
}

// --------------------------------------------------------------------------------------------
// QIP case 8: cross-chain vector
// --------------------------------------------------------------------------------------------

#[test]
fn case8_cross_chain_vector_matches_solidity_sha256_bytes32() {
	// The runtime hash and an independent SHA-256 implementation agree with the Foundry value.
	assert_eq!(sp_io::hashing::sha2_256(&S), H);
	assert_eq!(<[u8; 32]>::from(Sha256::digest(S)), H);

	new_test_ext().execute_with(|| {
		// The same S that claims QubiHTLC.sol (verified with Foundry) claims the Quantus lock.
		let (id, _) = lock_default();
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(CHARLIE), id, S));
		assert_eq!(last_event(), RuntimeEvent::Htlc(Event::Claimed { id, preimage: S }));
		assert_eq!(free(BOB), 1_000 + AMOUNT);
	});
}

// --------------------------------------------------------------------------------------------
// misc
// --------------------------------------------------------------------------------------------

#[test]
fn integrity_test_passes() {
	new_test_ext().execute_with(|| {
		use frame_support::traits::Hooks;
		<Htlc as Hooks<u64>>::integrity_test();
	});
}

#[test]
fn locks_are_independent() {
	new_test_ext().execute_with(|| {
		let (a, _) = lock_default();
		let (b, expiry_b) = lock_with(ALICE, CHARLIE, 2 * AMOUNT, now() + MIN_DURATION * 5);
		assert_eq!(held(HoldReason::HtlcLock, ALICE), 3 * AMOUNT);
		assert_eq!(held(HoldReason::HtlcDeposit, ALICE), 2 * DEPOSIT);
		assert_ok!(Htlc::claim(RuntimeOrigin::signed(BOB), a, S));
		assert_eq!(held(HoldReason::HtlcLock, ALICE), 2 * AMOUNT);
		assert_eq!(held(HoldReason::HtlcDeposit, ALICE), DEPOSIT);
		System::set_block_number(expiry_b);
		assert_ok!(Htlc::refund(RuntimeOrigin::signed(BOB), b));
		assert_eq!(free(ALICE), 10_000 - AMOUNT);
		assert_eq!(free(CHARLIE), 1_000);
		assert_eq!(LockCount::<Test>::get(ALICE), 0);
	});
}
