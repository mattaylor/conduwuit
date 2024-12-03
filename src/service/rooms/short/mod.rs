use std::{borrow::Borrow, fmt::Debug, mem::size_of_val, sync::Arc};

pub use conduit::pdu::{ShortEventId, ShortId, ShortRoomId};
use conduit::{err, implement, utils, utils::stream::ReadyExt, Result};
use database::{Deserialized, Map};
use futures::{Stream, StreamExt};
use ruma::{events::StateEventType, EventId, RoomId};
use serde::Deserialize;

use crate::{globals, Dep};

pub struct Service {
	db: Data,
	services: Services,
}

struct Data {
	eventid_shorteventid: Arc<Map>,
	shorteventid_eventid: Arc<Map>,
	statekey_shortstatekey: Arc<Map>,
	shortstatekey_statekey: Arc<Map>,
	roomid_shortroomid: Arc<Map>,
	statehash_shortstatehash: Arc<Map>,
}

struct Services {
	globals: Dep<globals::Service>,
}

pub type ShortStateHash = ShortId;
pub type ShortStateKey = ShortId;

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data {
				eventid_shorteventid: args.db["eventid_shorteventid"].clone(),
				shorteventid_eventid: args.db["shorteventid_eventid"].clone(),
				statekey_shortstatekey: args.db["statekey_shortstatekey"].clone(),
				shortstatekey_statekey: args.db["shortstatekey_statekey"].clone(),
				roomid_shortroomid: args.db["roomid_shortroomid"].clone(),
				statehash_shortstatehash: args.db["statehash_shortstatehash"].clone(),
			},
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
pub async fn get_or_create_shorteventid(&self, event_id: &EventId) -> ShortEventId {
	if let Ok(shorteventid) = self.get_shorteventid(event_id).await {
		return shorteventid;
	}

	self.create_shorteventid(event_id)
}

#[implement(Service)]
pub fn multi_get_or_create_shorteventid<'a, I>(&'a self, event_ids: I) -> impl Stream<Item = ShortEventId> + Send + '_
where
	I: Iterator<Item = &'a EventId> + Clone + Debug + ExactSizeIterator + Send + 'a,
	<I as Iterator>::Item: AsRef<[u8]> + Send + Sync + 'a,
{
	self.db
		.eventid_shorteventid
		.get_batch(event_ids.clone())
		.ready_scan(event_ids, |event_ids, result| {
			event_ids.next().map(|event_id| (event_id, result))
		})
		.map(|(event_id, result)| match result {
			Ok(ref short) => utils::u64_from_u8(short),
			Err(_) => self.create_shorteventid(event_id),
		})
}

#[implement(Service)]
fn create_shorteventid(&self, event_id: &EventId) -> ShortEventId {
	const BUFSIZE: usize = size_of::<ShortEventId>();

	let short = self.services.globals.next_count().unwrap();
	debug_assert!(size_of_val(&short) == BUFSIZE, "buffer requirement changed");

	self.db
		.eventid_shorteventid
		.raw_aput::<BUFSIZE, _, _>(event_id, short);
	self.db
		.shorteventid_eventid
		.aput_raw::<BUFSIZE, _, _>(short, event_id);

	short
}

#[implement(Service)]
pub async fn get_shorteventid(&self, event_id: &EventId) -> Result<ShortEventId> {
	self.db
		.eventid_shorteventid
		.get(event_id)
		.await
		.deserialized()
}

#[implement(Service)]
pub async fn get_or_create_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> ShortStateKey {
	const BUFSIZE: usize = size_of::<ShortStateKey>();

	if let Ok(shortstatekey) = self.get_shortstatekey(event_type, state_key).await {
		return shortstatekey;
	}

	let key = (event_type, state_key);
	let shortstatekey = self.services.globals.next_count().unwrap();
	debug_assert!(size_of_val(&shortstatekey) == BUFSIZE, "buffer requirement changed");

	self.db
		.statekey_shortstatekey
		.put_aput::<BUFSIZE, _, _>(key, shortstatekey);

	self.db
		.shortstatekey_statekey
		.aput_put::<BUFSIZE, _, _>(shortstatekey, key);

	shortstatekey
}

#[implement(Service)]
pub async fn get_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> Result<ShortStateKey> {
	let key = (event_type, state_key);
	self.db
		.statekey_shortstatekey
		.qry(&key)
		.await
		.deserialized()
}

#[implement(Service)]
pub async fn get_eventid_from_short<Id>(&self, shorteventid: ShortEventId) -> Result<Id>
where
	Id: for<'de> Deserialize<'de> + Sized + ToOwned,
	<Id as ToOwned>::Owned: Borrow<EventId>,
{
	const BUFSIZE: usize = size_of::<ShortEventId>();

	self.db
		.shorteventid_eventid
		.aqry::<BUFSIZE, _>(&shorteventid)
		.await
		.deserialized()
		.map_err(|e| err!(Database("Failed to find EventId from short {shorteventid:?}: {e:?}")))
}

#[implement(Service)]
pub fn multi_get_eventid_from_short<'a, Id, I>(&'a self, shorteventid: I) -> impl Stream<Item = Result<Id>> + Send + 'a
where
	I: Iterator<Item = &'a ShortEventId> + Send + 'a,
	Id: for<'de> Deserialize<'de> + Sized + ToOwned + 'a,
	<Id as ToOwned>::Owned: Borrow<EventId>,
{
	const BUFSIZE: usize = size_of::<ShortEventId>();

	self.db
		.shorteventid_eventid
		.aqry_batch::<BUFSIZE, _, _>(shorteventid)
		.map(Deserialized::deserialized)
}

#[implement(Service)]
pub async fn get_statekey_from_short(&self, shortstatekey: ShortStateKey) -> Result<(StateEventType, String)> {
	const BUFSIZE: usize = size_of::<ShortStateKey>();

	self.db
		.shortstatekey_statekey
		.aqry::<BUFSIZE, _>(&shortstatekey)
		.await
		.deserialized()
		.map_err(|e| {
			err!(Database(
				"Failed to find (StateEventType, state_key) from short {shortstatekey:?}: {e:?}"
			))
		})
}

/// Returns (shortstatehash, already_existed)
#[implement(Service)]
pub async fn get_or_create_shortstatehash(&self, state_hash: &[u8]) -> (ShortStateHash, bool) {
	const BUFSIZE: usize = size_of::<ShortStateHash>();

	if let Ok(shortstatehash) = self
		.db
		.statehash_shortstatehash
		.get(state_hash)
		.await
		.deserialized()
	{
		return (shortstatehash, true);
	}

	let shortstatehash = self.services.globals.next_count().unwrap();
	debug_assert!(size_of_val(&shortstatehash) == BUFSIZE, "buffer requirement changed");

	self.db
		.statehash_shortstatehash
		.raw_aput::<BUFSIZE, _, _>(state_hash, shortstatehash);

	(shortstatehash, false)
}

#[implement(Service)]
pub async fn get_shortroomid(&self, room_id: &RoomId) -> Result<ShortRoomId> {
	self.db.roomid_shortroomid.get(room_id).await.deserialized()
}

#[implement(Service)]
pub async fn get_or_create_shortroomid(&self, room_id: &RoomId) -> ShortRoomId {
	self.db
		.roomid_shortroomid
		.get(room_id)
		.await
		.deserialized()
		.unwrap_or_else(|_| {
			const BUFSIZE: usize = size_of::<ShortRoomId>();

			let short = self.services.globals.next_count().unwrap();
			debug_assert!(size_of_val(&short) == BUFSIZE, "buffer requirement changed");

			self.db
				.roomid_shortroomid
				.raw_aput::<BUFSIZE, _, _>(room_id, short);

			short
		})
}
