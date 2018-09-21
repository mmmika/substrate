// Copyright 2017 Parity Technologies (UK) Ltd.
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

//! Conensus module for runtime; manages the authority set ready for the native code.

#![cfg_attr(not(feature = "std"), no_std)]

#[allow(unused_imports)]
#[macro_use]
extern crate sr_std as rstd;

#[macro_use]
extern crate srml_support as runtime_support;

#[cfg(feature = "std")]
#[macro_use]
extern crate serde_derive;

extern crate sr_io as runtime_io;
extern crate parity_codec as codec;
#[macro_use]
extern crate parity_codec_derive;
extern crate srml_system as system;
extern crate sr_primitives as primitives;
extern crate substrate_primitives;

use rstd::prelude::*;
use runtime_support::{storage, Parameter};
use runtime_support::dispatch::Result;
use runtime_support::storage::StorageValue;
use runtime_support::storage::unhashed::StorageVec;
use primitives::traits::{MaybeSerializeDebug, OnFinalise, Member, DigestItem};
use system::{ensure_signed, ensure_inherent, ensure_root};

#[cfg(any(feature = "std", test))]
use std::collections::HashMap;


pub const AUTHORITY_AT: &'static [u8] = b":auth:";
pub const AUTHORITY_COUNT: &'static [u8] = b":auth:len";

mod mock;
mod tests;

struct AuthorityStorageVec<S: codec::Codec + Default>(rstd::marker::PhantomData<S>);
impl<S: codec::Codec + Default> StorageVec for AuthorityStorageVec<S> {
	type Item = S;
	const PREFIX: &'static [u8] = AUTHORITY_AT;
}

pub const CODE: &'static [u8] = b":code";

pub type KeyValue = (Vec<u8>, Vec<u8>);

pub trait OnOfflineValidator {
	fn on_offline_validator(validator_index: usize);
}

impl OnOfflineValidator for () {
	fn on_offline_validator(_validator_index: usize) {}
}

pub type Log<T> = RawLog<
	<T as Trait>::SessionKey,
>;

/// A logs in this module.
#[cfg_attr(feature = "std", derive(Serialize, Deserialize, Debug))]
#[derive(Encode, Decode, PartialEq, Eq, Clone)]
pub enum RawLog<SessionKey> {
	/// Authorities set has been changed. Contains the new set of authorities.
	AuthoritiesChange(Vec<SessionKey>),
}

impl<SessionKey: Member> DigestItem for RawLog<SessionKey> {
	type Hash = ::substrate_primitives::H256;
	type AuthorityId = SessionKey;

	/// Try to cast the log entry as AuthoritiesChange log entry.
	fn as_authorities_change(&self) -> Option<&[SessionKey]> {
		match *self {
			RawLog::AuthoritiesChange(ref item) => Some(&item),
		}
	}
}

// Implementation for tests outside of this crate.
#[cfg(any(feature = "std", test))]
impl<N> From<RawLog<N>> for primitives::testing::DigestItem where N: Into<u64> {
	fn from(log: RawLog<N>) -> primitives::testing::DigestItem {
		match log {
			RawLog::AuthoritiesChange(authorities) =>
				primitives::generic::DigestItem::AuthoritiesChange
					::<substrate_primitives::H256, u64>(authorities.into_iter()
						.map(Into::into).collect()),
		}
	}
}

pub trait Trait: system::Trait {
	/// The allowed extrinsic position for `note_offline` inherent.
	const NOTE_OFFLINE_POSITION: u32;

	/// Type for all log entries of this module.
	type Log: From<Log<Self>> + Into<system::DigestItemOf<Self>>;

	type SessionKey: Parameter + Default + MaybeSerializeDebug;
	type OnOfflineValidator: OnOfflineValidator;
}

decl_storage! {
	trait Store for Module<T: Trait> as Consensus {
		// Authorities set actual at the block execution start. IsSome only if
		// the set has been changed.
		OriginalAuthorities: Vec<T::SessionKey>;
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn report_misbehavior(origin, report: Vec<u8>) -> Result;
		fn note_offline(origin, offline_val_indices: Vec<u32>) -> Result;
		fn remark(origin, remark: Vec<u8>) -> Result;
		fn set_code(origin, new: Vec<u8>) -> Result;
		fn set_storage(origin, items: Vec<KeyValue>) -> Result;
	}
}

impl<T: Trait> Module<T> {
	/// Get the current set of authorities. These are the session keys.
	pub fn authorities() -> Vec<T::SessionKey> {
		AuthorityStorageVec::<T::SessionKey>::items()
	}

	/// Set the new code.
	fn set_code(origin: T::Origin, new: Vec<u8>) -> Result {
		ensure_root(origin)?;
		storage::unhashed::put_raw(CODE, &new);
		Ok(())
	}

	/// Set some items of storage.
	fn set_storage(origin: T::Origin, items: Vec<KeyValue>) -> Result {
		ensure_root(origin)?;
		for i in &items {
			storage::unhashed::put_raw(&i.0, &i.1);
		}
		Ok(())
	}

	/// Report some misbehaviour.
	fn report_misbehavior(origin: T::Origin, _report: Vec<u8>) -> Result {
		ensure_signed(origin)?;
		// TODO.
		Ok(())
	}

	/// Note the previous block's validator missed their opportunity to propose a block. This only comes in
	/// if 2/3+1 of the validators agree that no proposal was submitted. It's only relevant
	/// for the previous block.
	fn note_offline(origin: T::Origin, offline_val_indices: Vec<u32>) -> Result {
		ensure_inherent(origin)?;
		assert!(
			<system::Module<T>>::extrinsic_index() == Some(T::NOTE_OFFLINE_POSITION),
			"note_offline extrinsic must be at position {} in the block",
			T::NOTE_OFFLINE_POSITION
		);

		for validator_index in offline_val_indices.into_iter() {
			T::OnOfflineValidator::on_offline_validator(validator_index as usize);
		}
		
		Ok(())
	}

	/// Make some on-chain remark.
	fn remark(origin: T::Origin, _remark: Vec<u8>) -> Result {
		ensure_signed(origin)?;
		Ok(())
	}

	/// Set the current set of authorities' session keys.
	///
	/// Called by `next_session` only.
	pub fn set_authorities(authorities: &[T::SessionKey]) {
		let current_authorities = AuthorityStorageVec::<T::SessionKey>::items();
		if current_authorities != authorities {
			Self::save_original_authorities(Some(current_authorities));
			AuthorityStorageVec::<T::SessionKey>::set_items(authorities);
		}
	}

	/// Set a single authority by index.
	pub fn set_authority(index: u32, key: &T::SessionKey) {
		let current_authority = AuthorityStorageVec::<T::SessionKey>::item(index);
		if current_authority != *key {
			Self::save_original_authorities(None);
			AuthorityStorageVec::<T::SessionKey>::set_item(index, key);
		}
	}

	/// Save original authorities set.
	fn save_original_authorities(current_authorities: Option<Vec<T::SessionKey>>) {
		if OriginalAuthorities::<T>::get().is_some() {
			// if we have already saved original set before, do not overwrite
			return;
		}

		<OriginalAuthorities<T>>::put(current_authorities.unwrap_or_else(||
			AuthorityStorageVec::<T::SessionKey>::items()));
	}

	/// Deposit one of this module's logs.
	fn deposit_log(log: Log<T>) {
		<system::Module<T>>::deposit_log(<T as Trait>::Log::from(log).into());
	}
}

/// Finalization hook for the consensus module.
impl<T: Trait> OnFinalise<T::BlockNumber> for Module<T> {
	fn on_finalise(_n: T::BlockNumber) {
		if let Some(original_authorities) = <OriginalAuthorities<T>>::take() {
			let current_authorities = AuthorityStorageVec::<T::SessionKey>::items();
			if current_authorities != original_authorities {
				Self::deposit_log(RawLog::AuthoritiesChange(current_authorities));
			}
		}
	}
}

#[cfg(any(feature = "std", test))]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct GenesisConfig<T: Trait> {
	pub authorities: Vec<T::SessionKey>,
	#[serde(with = "substrate_primitives::bytes")]
	pub code: Vec<u8>,
}

#[cfg(any(feature = "std", test))]
impl<T: Trait> Default for GenesisConfig<T> {
	fn default() -> Self {
		GenesisConfig {
			authorities: vec![],
			code: vec![],
		}
	}
}

#[cfg(any(feature = "std", test))]
impl<T: Trait> primitives::BuildStorage for GenesisConfig<T>
{
	fn build_storage(self) -> ::std::result::Result<HashMap<Vec<u8>, Vec<u8>>, String> {
		use codec::{Encode, KeyedVec};
		use substrate_primitives::Blake2Hasher;
		let auth_count = self.authorities.len() as u32;
		let mut r: runtime_io::TestExternalities<Blake2Hasher> = self.authorities.into_iter().enumerate().map(|(i, v)|
			((i as u32).to_keyed_vec(AUTHORITY_AT), v.encode())
		).collect();
		r.insert(AUTHORITY_COUNT.to_vec(), auth_count.encode());
		r.insert(CODE.to_vec(), self.code);
		Ok(r.into())
	}
}
