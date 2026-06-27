use libstrophe::Stanza;

use crate::{config::Webrtc, presence::ParticipantPresence, room::Room};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc::Sender},
};

pub type Rooms = Arc<Mutex<RoomManager>>;

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

    // pub fn on_meeting_started(&self, name: &str) {
    //     if let Some(room) = self.get(name) {
    //         room.on_meeting_started();
    //     }
    // }

    pub fn on_participant_joined(
        &mut self,
        name: &str,
        tx: Sender<Stanza>,
        webrtc: &Webrtc,
        participant: ParticipantPresence,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.contains_key(name) {
            let room = Room::new(name.to_string(), tx.clone(), &webrtc)?;
            self.insert(room);
        }

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
                    participant.video_muted,
                    participant.audio_muted,
                    participant.screenshare_muted,
                );
            }
        }
        Ok(())
    }

    pub fn on_participant_left(&mut self, name: &str, endpoint_id: &str) {
        if let Some(room) = self.get_mut(name) {
            room.on_participant_left(endpoint_id);
        }
    }

    pub fn on_meeting_terminated(&mut self, name: &str) {
        if let Some(room) = self.rooms.get_mut(name) {
            room.on_meeting_terminated();
        }
        // self.rooms.remove(name);
    }
}
