pub(crate) mod jingle_action;
pub(crate) mod jingle_media;

use crate::{
    get_attribute,
    iq::{jingle_action::JingleAction, jingle_media::JingleMedia},
    room::Room,
};
use libstrophe::Stanza;

#[derive(Debug)]
#[allow(unused)]
pub(crate) struct Iq {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    jingle_media: JingleMedia,
}

impl Iq {
    pub fn new(stanza: &Stanza) -> Self {
        let iq_stanza = get_attribute!(stanza, {
            from => "from",
            to => "to",
            id => "id",
            kind => "type"
        });

        let media = JingleMedia::new();

        Self {
            id: iq_stanza.id,
            from: iq_stanza.from,
            to: iq_stanza.to,
            kind: iq_stanza.kind,
            jingle_media: media,
        }
    }

    pub fn handle_jingle(&mut self, stanza: &Stanza, room: Option<&mut Room>) {
        let jingle_stanza = get_attribute!(stanza, [sid, initiator, action]);
        let jingle_action = JingleAction::parse(&jingle_stanza.action, stanza);
        if let Some(ref action) = jingle_action {
            match action {
                JingleAction::SessionInitiate(stanza) => {
                    let jitsi_offer =
                        action.handle_session_initiate(stanza, &mut self.jingle_media);

                    if let Some(room) = room {}
                }
                JingleAction::SourceAdd(stanza) => {
                    action.handle_source_add(stanza);
                }
            }
        }
    }

    pub fn handle_query(&self, _stanza: &Stanza) {}
}
