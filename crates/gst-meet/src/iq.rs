pub(crate) mod jingle_action;
pub(crate) mod jingle_media;

use crate::{
    get_attribute,
    iq::{jingle_action::JingleAction, jingle_media::JingleMedia},
};
use libstrophe::Stanza;

#[derive(Debug)]
#[allow(unused)]
pub(crate) struct Iq<'a> {
    id: String,
    from: String,
    to: String,
    kind: String,
    jingle_action: Option<JingleAction<'a>>,
    jingle_media: JingleMedia,
}

impl<'a> Iq<'a> {
    pub fn new(stanza: &'a Stanza) -> Self {
        let iq_stanza = get_attribute!(stanza, {
            from => "from",
            to => "to",
            id => "id",
            kind => "type"
        });

        let jingle_stanza = get_attribute!(stanza, [sid, initiator, action]);

        let media = JingleMedia::new();
        let jingle_action = JingleAction::parse(&jingle_stanza.action, stanza);

        Self {
            id: iq_stanza.id,
            from: iq_stanza.from,
            to: iq_stanza.to,
            kind: iq_stanza.kind,
            jingle_action,
            jingle_media: media,
        }
    }

    pub fn handle_jingle(&mut self) {
        if let Some(ref action) = self.jingle_action {
            action.handle(&mut self.jingle_media);
        }
    }

    pub fn handle_query(&self, _stanza: &Stanza) {}
}
