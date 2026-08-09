//! Macros for building XMPP stanzas and for working with the weak references
//! held by GStreamer callbacks.

pub mod stanza;

/// Upgrades a [`RoomWeak`](crate::room::RoomWeak)-style handle or returns early.
///
/// GStreamer signal handlers outlive the objects they capture, so they hold a
/// weak reference and bail out once the room is gone:
///
/// ```ignore
/// let room = upgrade_weak!(room_clone);        // returns () if dropped
/// let room = upgrade_weak!(room_clone, Ok(())); // or a custom value
/// ```
#[macro_export]
macro_rules! upgrade_weak {
    ($x:ident, $r:expr) => {{
        match $x.upgrade() {
            Some(o) => o,
            None => return $r,
        }
    }};
    ($x:ident) => {
        upgrade_weak!($x, ())
    };
}
