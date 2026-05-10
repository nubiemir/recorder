use crate::room::Room;
use std::collections::HashMap;

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
        self.rooms
            .entry(room.get_name().to_string())
            .or_insert_with(|| room);
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Room> {
        self.rooms.get_mut(name)
    }
}
