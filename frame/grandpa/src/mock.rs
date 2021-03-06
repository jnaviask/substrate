// Copyright 2018-2019 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! Test utilities

#![cfg(test)]

use sp_runtime::{Perbill, DigestItem, traits::IdentityLookup, testing::{Header, UintAuthorityId}};
use sp_io;
use support::{impl_outer_origin, impl_outer_event, parameter_types, weights::Weight};
use primitives::H256;
use codec::{Encode, Decode};
use crate::{AuthorityId, AuthorityList, GenesisConfig, Trait, Module, ConsensusLog};
use sp_finality_grandpa::GRANDPA_ENGINE_ID;

impl_outer_origin!{
	pub enum Origin for Test {}
}

pub fn grandpa_log(log: ConsensusLog<u64>) -> DigestItem<H256> {
	DigestItem::Consensus(GRANDPA_ENGINE_ID, log.encode())
}

// Workaround for https://github.com/rust-lang/rust/issues/26925 . Remove when sorted.
#[derive(Clone, PartialEq, Eq, Debug, Decode, Encode)]
pub struct Test;

impl Trait for Test {
	type Event = TestEvent;
}
parameter_types! {
	pub const BlockHashCount: u64 = 250;
	pub const MaximumBlockWeight: Weight = 1024;
	pub const MaximumBlockLength: u32 = 2 * 1024;
	pub const AvailableBlockRatio: Perbill = Perbill::one();
}
impl system::Trait for Test {
	type Origin = Origin;
	type Index = u64;
	type BlockNumber = u64;
	type Call = ();
	type Hash = H256;
	type Hashing = sp_runtime::traits::BlakeTwo256;
	type AccountId = u64;
	type Lookup = IdentityLookup<Self::AccountId>;
	type Header = Header;
	type Event = TestEvent;
	type BlockHashCount = BlockHashCount;
	type MaximumBlockWeight = MaximumBlockWeight;
	type MaximumBlockLength = MaximumBlockLength;
	type AvailableBlockRatio = AvailableBlockRatio;
	type Version = ();
}

mod grandpa {
	pub use crate::Event;
}

impl_outer_event!{
	pub enum TestEvent for Test {
		grandpa,
	}
}

pub fn to_authorities(vec: Vec<(u64, u64)>) -> AuthorityList {
	vec.into_iter()
		.map(|(id, weight)| (UintAuthorityId(id).to_public_key::<AuthorityId>(), weight))
		.collect()
}

pub fn new_test_ext(authorities: Vec<(u64, u64)>) -> sp_io::TestExternalities {
	let mut t = system::GenesisConfig::default().build_storage::<Test>().unwrap();
	GenesisConfig {
		authorities: to_authorities(authorities),
	}.assimilate_storage::<Test>(&mut t).unwrap();
	t.into()
}

pub type System = system::Module<Test>;
pub type Grandpa = Module<Test>;
