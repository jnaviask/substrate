// Copyright 2019 Parity Technologies (UK) Ltd.
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

//! Gossip and politeness for GRANDPA communication.

use runtime_primitives::traits::Block as BlockT;
use network::consensus_gossip as network_gossip;
use parity_codec::{Encode, Decode};

use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};
use log::debug;

use crate::{CompactCommit, SignedMessage};

/// An outcome of examining a message.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Consider {
	/// Accept the message.
	Accept,
	/// Message is too early. Reject.
	RejectPast,
	/// Message is from the future. Reject.
	RejectFuture,
}

/// A view of protocol state.
pub struct View {
	round: u64, // the current round we are at.
	set_id: u64, // the current voter set id.
}

impl View {
	/// Update the set ID. implies a reset to round 0.
	pub fn update_set(&mut self, set_id: u64) {
		self.set_id = set_id;
		self.round = 0;
	}

	/// Consider a round and set ID combination under a current view.
	pub fn consider(&self, round: u64, set_id: u64) -> Consider {
		// only from current set
		if set_id < self.set_id { return Consider::RejectPast }
		if set_id > self.set_id { return Consider::RejectFuture }

		// only r-1 ... r+1
		if round > self.round.saturating_add(1) { return Consider::RejectFuture }
		if round < self.round.saturating_sub(1) { return Consider::RejectPast }

		Consider::Accept
	}
}

/// Grandpa gossip message type.
/// This is the root type that gets encoded and sent on the network.
#[derive(Debug, Encode, Decode)]
pub enum GossipMessage<Block: BlockT> {
	/// Grandpa message with round and set info.
	VoteOrPrecommit(VoteOrPrecommitMessage<Block>),
	/// Grandpa commit message with round and set info.
	Commit(FullCommitMessage<Block>),
}

/// Network level message with topic information.
#[derive(Debug, Encode, Decode)]
pub struct VoteOrPrecommitMessage<Block: BlockT> {
	/// The round this message is from.
	pub round: u64,
	/// The voter set ID this message is from.
	pub set_id: u64,
	/// The message itself.
	pub message: SignedMessage<Block>,
}

/// Network level commit message with topic information.
#[derive(Debug, Encode, Decode)]
pub struct FullCommitMessage<Block: BlockT> {
	/// The round this message is from.
	pub round: u64,
	/// The voter set ID this message is from.
	pub set_id: u64,
	/// The compact commit message.
	pub message: CompactCommit<Block>,
}

/// A validator for GRANDPA gossip messages.
pub(crate) struct GossipValidator<Block: BlockT> {
	local_view: parking_lot::RwLock<View>,
	_marker: std::marker::PhantomData<Block>,
}

impl<Block: BlockT> GossipValidator<Block> {
	/// Create a new gossip-validator.
	pub(crate) fn new() -> GossipValidator<Block> {
		GossipValidator {
			local_view: parking_lot::RwLock::new(View {
				round: 0,
				set_id: 0,
			}),
			_marker: Default::default(),
		}
	}

	/// Note a round in a set has started.
	pub(crate) fn note_round(&self, round: u64, set_id: u64) {
		let mut view = self.local_view.write();
		*view = View { round, set_id };
	}

	/// Note that a voter set with given ID has started.
	pub(crate) fn note_set(&self, set_id: u64) {
		self.local_view.write().update_set(set_id);
	}

	/// Consider a message.
	fn consider(&self, round: u64, set_id: u64) -> Consider {
		self.local_view.read().consider(round, set_id)
	}

	fn validate_round_message(&self, full: VoteOrPrecommitMessage<Block>)
		-> network_gossip::ValidationResult<Block::Hash>
	{
		match self.consider(full.round, full.set_id) {
			Consider::RejectFuture | Consider::RejectPast =>
				return network_gossip::ValidationResult::Expired,
			_ => {},
		}

		if let Err(()) = super::check_message_sig::<Block>(
			&full.message.message,
			&full.message.id,
			&full.message.signature,
			full.round,
			full.set_id
		) {
			debug!(target: "afg", "Bad message signature {}", full.message.id);
			telemetry!(CONSENSUS_DEBUG; "afg.bad_msg_signature"; "signature" => ?full.message.id);
			return network_gossip::ValidationResult::Invalid;
		}

		let topic = super::message_topic::<Block>(full.round, full.set_id);
		network_gossip::ValidationResult::Valid(topic)
	}

	fn validate_commit_message(&self, full: FullCommitMessage<Block>)
		-> network_gossip::ValidationResult<Block::Hash>
	{
		use grandpa::Message as GrandpaMessage;

		match self.consider(full.round, full.set_id) {
			Consider::RejectFuture | Consider::RejectPast =>
				return network_gossip::ValidationResult::Expired,
			_ => {},
		}

		if full.message.precommits.len() != full.message.auth_data.len() || full.message.precommits.is_empty() {
			debug!(target: "afg", "Malformed compact commit");
			telemetry!(CONSENSUS_DEBUG; "afg.malformed_compact_commit";
				"precommits_len" => ?full.message.precommits.len(),
				"auth_data_len" => ?full.message.auth_data.len(),
				"precommits_is_empty" => ?full.message.precommits.is_empty(),
			);
			return network_gossip::ValidationResult::Invalid;
		}

		// check signatures on all contained precommits.
		for (precommit, &(ref sig, ref id)) in full.message.precommits.iter().zip(&full.message.auth_data) {
			if let Err(()) = super::check_message_sig::<Block>(
				&GrandpaMessage::Precommit(precommit.clone()),
				id,
				sig,
				full.round,
				full.set_id,
			) {
				debug!(target: "afg", "Bad commit message signature {}", id);
				telemetry!(CONSENSUS_DEBUG; "afg.bad_commit_msg_signature"; "id" => ?id);
				return network_gossip::ValidationResult::Invalid;
			}
		}

		let topic = super::commit_topic::<Block>(full.set_id);
		network_gossip::ValidationResult::Valid(topic)
	}
}

impl<Block: BlockT> network_gossip::Validator<Block::Hash> for GossipValidator<Block> {
	fn validate(&self, mut data: &[u8]) -> network_gossip::ValidationResult<Block::Hash> {
		match GossipMessage::<Block>::decode(&mut data) {
			Some(GossipMessage::VoteOrPrecommit(message)) => self.validate_round_message(message),
			Some(GossipMessage::Commit(message)) => self.validate_commit_message(message),
			None => {
				debug!(target: "afg", "Error decoding message");
				telemetry!(CONSENSUS_DEBUG; "afg.err_decoding_msg"; "" => "");
				network_gossip::ValidationResult::Invalid
			}
		}
	}

	fn message_expired<'a>(&'a self) -> Box<FnMut(Block::Hash, &[u8]) -> bool + 'a> {
		let rounds = self.local_view.read();
		Box::new(move |_topic, mut data| {
			let consider = match GossipMessage::<Block>::decode(&mut data) {
				None => Consider::Accept,
				Some(GossipMessage::Commit(full)) => rounds.consider(full.round, full.set_id),
				Some(GossipMessage::VoteOrPrecommit(full)) =>
					rounds.consider(full.round, full.set_id),
			};

			consider == Consider::Accept
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn view_accepts_current() {
		let view = View { round: 100, set_id: 1 };

		assert_eq!(view.consider(98, 1), Consider::RejectPast);
		assert_eq!(view.consider(1, 0), Consider::RejectPast);
		assert_eq!(view.consider(1000, 0), Consider::RejectPast);

		assert_eq!(view.consider(99, 1), Consider::Accept);
		assert_eq!(view.consider(100, 1), Consider::Accept);
		assert_eq!(view.consider(101, 1), Consider::Accept);

		assert_eq!(view.consider(102, 1), Consider::RejectFuture);
		assert_eq!(view.consider(1, 2), Consider::RejectFuture);
		assert_eq!(view.consider(1000, 2), Consider::RejectFuture);
	}
}