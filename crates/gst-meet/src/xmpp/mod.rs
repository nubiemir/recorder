use libstrophe::{Error, Stanza};

pub mod xep;

pub fn ack_session_initiate(iq_stanza: &Stanza) -> Result<Stanza, Error> {
    let id = iq_stanza.get_attribute("id");
    let to = iq_stanza.get_attribute("to").unwrap_or_default();
    let from = iq_stanza.get_attribute("from").unwrap_or_default();
    let mut iq = Stanza::new_iq(Some("result"), id);
    iq.set_attribute("to", from)?;
    iq.set_attribute("from", to)?;

    return Ok(iq);
}
