//! Routes presence events to the right [`Room`], creating and retiring rooms
//! as meetings start and end.

use gstreamer::glib::BoolError;
use libstrophe::Stanza;

use crate::{
    config::Webrtc,
    presence::{Presence, participant_presence::ParticipantPresence},
    room::Room,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc::Sender},
};

/// Shared handle to the manager, held by the XMPP callbacks that dispatch into
/// it from the connection thread.
pub type Rooms = Arc<Mutex<RoomManager>>;

/// Every meeting the recorder is currently in, keyed by room name.
#[derive(Debug)]
pub struct RoomManager {
    rooms: HashMap<String, Room>,
}

impl RoomManager {
    pub fn new() -> Self {
        RoomManager {
            rooms: HashMap::new(),
        }
    }

    /// Adds a room unless one with the same name is already tracked.
    pub fn insert(&mut self, room: Room) {
        self.rooms.entry(room.name.clone()).or_insert_with(|| room);
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Room> {
        self.rooms.get_mut(name)
    }
    pub fn get(&self, name: &str) -> Option<&Room> {
        self.rooms.get(name)
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.rooms.contains_key(name)
    }

    /// Creates the room and its pipeline ahead of joining, so the presence
    /// reply and the Jingle offer that follow have something to land on.
    ///
    /// Idempotent: joining a room the recorder is already in is a no-op.
    pub fn handle_join_room(
        &mut self,
        name: &str,
        tx: Sender<Stanza>,
        webrtc: &Webrtc,
        presence: Presence,
    ) -> Result<(), BoolError> {
        if !self.contains_key(name) {
            let room = Room::new(name.to_string(), tx, webrtc, presence)?;
            self.insert(room);
        }

        Ok(())
    }

    /// Handles available presence, creating the room (and its pipeline) on the
    /// first participant seen.
    ///
    /// Jitsi reuses presence for mute updates, so this splits the two cases:
    /// an endpoint we haven't seen is a real join, anything else is a
    /// `SourceInfo` update on an existing participant.
    pub fn on_participant_joined(
        &mut self,
        name: &str,
        participant: ParticipantPresence,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(room) = self.get_mut(name) {
            if !room.endpoint_available(&participant.endpoint_id) {
                room.on_participant_joined(
                    &participant.endpoint_id,
                    &participant.display_name.unwrap_or_default(),
                    participant.video_muted,
                    participant.audio_muted,
                    participant.screenshare_muted,
                );
            } else {
                room.source_info_updated(
                    &participant.endpoint_id,
                    &participant.display_name.unwrap_or_default(),
                    participant.video_muted,
                    participant.audio_muted,
                    participant.screenshare_muted,
                );
            }
        }
        Ok(())
    }

    /// Finalizes whatever the departing participant was still recording.
    pub fn on_participant_left(&mut self, name: &str, endpoint_id: &str) {
        if let Some(room) = self.get_mut(name) {
            room.on_participant_left(endpoint_id);
        }
    }

    /// Starts the room draining and stops tracking it. Draining continues in
    /// the background; see the note below on why dropping here is safe.
    pub fn on_meeting_terminated(&mut self, name: &str) {
        if let Some(room) = self.rooms.get_mut(name) {
            room.on_meeting_terminated();
        }
        // The room's bus watcher thread holds a strong Room and keeps it
        // alive until draining finishes, so it is safe to drop it here.
        self.rooms.remove(name);
    }
}
