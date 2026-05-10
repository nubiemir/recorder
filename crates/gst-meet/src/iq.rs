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

    pub fn handle_jingle_to_sdp(&mut self) {
        if let Some(ref action) = self.jingle_action {
            match action {
                JingleAction::SessionInitiate(stanza) => {
                    let _jitsi_offer =
                        action.handle_session_initiate(stanza, &mut self.jingle_media);
                }
                JingleAction::SourceAdd(stanza) => {
                    action.handle_source_add(stanza);
                }
            }
        }
    }

    pub fn handle_query_to_query(&self, _stanza: &Stanza) {}
}
