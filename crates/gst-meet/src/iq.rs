use crate::get_attributes;
use libstrophe::Stanza;
use std::fmt::Display;

#[derive(Debug)]
#[allow(unused)]
pub(crate) struct Iq {
    id: String,
    from: String,
    to: String,
    kind: String,
}

#[derive(Debug)]
enum JingleAction<'a> {
    SessionInitiate(&'a Stanza),
    SourceAdd(&'a Stanza),
}

impl Iq {
    pub fn new(stanza: &Stanza) -> Self {
        let iq_stanza = get_attributes!(stanza, {
            from => "from",
            to => "to",
            id => "id",
            kind => "type"
        });

        Self {
            id: iq_stanza.id,
            from: iq_stanza.from,
            to: iq_stanza.to,
            kind: iq_stanza.kind,
        }
    }

    pub fn handle_jingle(&mut self, stanza: &Stanza) {
        let jingle_stanza = get_attributes!(stanza, [sid, initiator, action]);
        if let Some(action) = JingleAction::parse(&jingle_stanza.action, stanza) {
            action.handle();
        }
    }

    pub fn handle_query(&self, _stanza: &Stanza) {}
}

impl<'a> JingleAction<'a> {
    fn parse(s: &str, stanza: &'a Stanza) -> Option<Self> {
        match s {
            "session-initiate" => Some(Self::SessionInitiate(stanza)),
            "source-add" => Some(Self::SourceAdd(stanza)),
            _ => None,
        }
    }

    fn handle(&self) {
        match self {
            Self::SessionInitiate(stanza) => {
                self.handle_session_initiate(stanza);
            }
            Self::SourceAdd(stanza) => {
                self.handle_source_add(stanza);
            }
        }
    }

    fn handle_session_initiate(&self, _stanza: &Stanza) {}
    fn handle_source_add(&self, _stanza: &Stanza) {}
}

impl<'a> Display for JingleAction<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionInitiate(_) => write!(f, "session-initiate"),
            Self::SourceAdd(_) => write!(f, "source-add"),
        }
    }
}
