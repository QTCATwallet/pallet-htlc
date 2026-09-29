//! Mock runtime for `pallet_htlc` tests (uses the chain's vendored `pallet_balances`).

use crate as pallet_htlc;
use frame_support::{derive_impl, parameter_types, traits::ConstU32};
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;
pub type Balance = u128;
pub type AccountId = u64;

pub const ALICE: AccountId = 1; // swap seller: locks QTC
pub const BOB: AccountId = 2; // swap buyer: recipient
pub const CHARLIE: AccountId = 3; // third party / relayer
pub const DAVE: AccountId = 4; // account that does not exist at genesis

pub const ED: Balance = 10;
pub const MIN_LOCK: Balance = 100;
pub const DEPOSIT: Balance = 5;
pub const MIN_DURATION: u64 = 10;
pub const MAX_DURATION: u64 = 1_000;
pub const MAX_LOCKS: u32 = 3;

#[frame_support::runtime]
mod runtime {
	use super::*;

	#[runtime::runtime]
	#[runtime::derive(
		RuntimeCall,
		RuntimeEvent,
		RuntimeError,
		RuntimeOrigin,
		RuntimeFreezeReason,
		RuntimeHoldReason,
		RuntimeSlashReason,
		RuntimeLockId,
		RuntimeTask
	)]
	pub struct Test;

	#[runtime::pallet_index(0)]
	pub type System = frame_system::Pallet<Test>;

	#[runtime::pallet_index(1)]
	pub type Balances = pallet_balances::Pallet<Test>;

	#[runtime::pallet_index(2)]
	pub type Htlc = pallet_htlc::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
	type AccountId = AccountId;
	type Lookup = sp_runtime::traits::IdentityLookup<Self::AccountId>;
	type AccountData = pallet_balances::AccountData<Balance>;
}

parameter_types! {
	pub const ExistentialDeposit: Balance = ED;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
	type Balance = Balance;
	type DustRemoval = ();
	type ExistentialDeposit = ExistentialDeposit;
	type AccountStore = frame_system::Pallet<Test>;
	type WeightInfo = ();
	type RuntimeHoldReason = RuntimeHoldReason;
	type RuntimeFreezeReason = RuntimeFreezeReason;
	// A plain id lets the tests place an arbitrary freeze (standing in for e.g. a vesting
	// schedule) on an account.
	type FreezeIdentifier = [u8; 8];
	type MaxFreezes = ConstU32<4>;
	type DoneSlashHandler = ();
}

parameter_types! {
	pub const MinLock: Balance = MIN_LOCK;
	pub const MinDuration: u64 = MIN_DURATION;
	pub const MaxDuration: u64 = MAX_DURATION;
	pub const MaxLocksPerAccount: u32 = MAX_LOCKS;
	pub const LockDeposit: Balance = DEPOSIT;
}

impl pallet_htlc::Config for Test {
	type RuntimeHoldReason = RuntimeHoldReason;
	type Currency = Balances;
	type MinLock = MinLock;
	type MinDuration = MinDuration;
	type MaxDuration = MaxDuration;
	type MaxLocksPerAccount = MaxLocksPerAccount;
	type LockDeposit = LockDeposit;
	type WeightInfo = ();
}

pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut t = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
	pallet_balances::GenesisConfig::<Test> {
		balances: vec![(ALICE, 10_000), (BOB, 1_000), (CHARLIE, 1_000)],
		..Default::default()
	}
	.assimilate_storage(&mut t)
	.unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}
