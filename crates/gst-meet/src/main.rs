use std::{
    ops::Deref,
    sync::{Arc, Mutex, mpsc::Sender},
    time::Duration,
};

use gst_meet::{gst::webrtcbin, jingle::from_jingle};
use libstrophe::{Connection, ConnectionEvent, ConnectionFlags, Context, HandlerResult, Stanza};
use log::{error, info, warn};

fn presence_handler() -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
    move |_ctx: &Context, _conn: &mut Connection, _stanza: &Stanza| HandlerResult::KeepHandler
}

fn iq_handler(
    tx: Sender<Stanza>,
) -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
    move |_ctx: &Context, _conn: &mut Connection, stanza: &Stanza| {
        // let to = stanza.get_attribute("to").unwrap_or_default();
        // let from = stanza.get_attribute("from").unwrap_or_default();
        let _id = stanza.get_attribute("id").unwrap_or_default();
        let _iq_type = stanza.get_attribute("type").unwrap_or_default();
        let child = stanza.get_first_child().unwrap();
        match child.name() {
            Some(c) => {
                if c == "jingle" {
                    let action = child.get_attribute("action").unwrap_or_default();
                    if action == "session-initiate" {
                        info!("got session initiate request");
                        match from_jingle(child.deref()) {
                            Ok(sdp) => {
                                info!("created sdp");
                                let res = webrtcbin(&sdp, tx.clone());
                                match res {
                                    Err(err) => warn!("Error occured: {:?}", err),
                                    _ => {}
                                }
                            }
                            Err(err) => {
                                warn!("Failed to generate sdp: {}", err);
                            }
                        }
                    }
                }
            }

            None => {}
        }
        // info!("iq stanza: {}", stanza.to_string());
        HandlerResult::KeepHandler
    }
}

fn join_muc(connection: &mut Connection) {
    let mut pres_stanza = Stanza::new();
    pres_stanza
        .set_name("presence")
        .expect("Failed to set presence name");
    pres_stanza
        .set_attribute("to", "testing123@muc.meet.jitsi/2")
        .expect("Failed to set to");

    let mut x = Stanza::new();
    x.set_name("x").expect("Failed to set x name");
    x.set_ns("http://jabber.org/protocol/muc")
        .expect("Failed to set ns");

    pres_stanza
        .add_child(x)
        .expect("Failed to append x to presence");

    connection.send(&pres_stanza);
    info!("Presence stanza sent");
}

fn main() {
    env_logger::init();
    gstreamer::init().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<Stanza>();
    let rx_shared = Arc::new(Mutex::new(rx));
    let connection_handler = move |_ctx: &Context, conn: &mut Connection, evt: ConnectionEvent| {
        match evt {
            ConnectionEvent::Connect => {
                info!("Success Connected");

                let iq_handler_tx = tx.clone();

                let rx_clone = Arc::clone(&rx_shared);
                conn.timed_handler_add(
                    move |_ctx, conn| {
                        while let Ok(stanza) = rx_clone.lock().unwrap().try_recv() {
                            info!("Sending stanza");
                            conn.send(&stanza);
                        }
                        HandlerResult::KeepHandler
                    },
                    Duration::from_millis(50),
                );

                conn.handler_add(presence_handler(), None, Some("presence"), None);
                conn.handler_add(iq_handler(iq_handler_tx), None, Some("iq"), None);
                join_muc(conn);
            }

            ConnectionEvent::Disconnect(conn_error) => {
                if let Some(err) = conn_error {
                    error!("++++++> Failed to Connect: {}", err);
                    // ctx.stop();
                } else {
                    error!("++++++> Failed to Connect: Something went wrong");
                    // ctx.stop();
                }
            }
            _ => {}
        }
    };

    let ctx = libstrophe::Context::new_with_default_logger();
    let mut conn = libstrophe::Connection::new(ctx);
    conn.set_jid("bot@hidden.meet.jitsi/");
    conn.set_pass("botpassword");
    let disable_tls = conn.set_flags(ConnectionFlags::DISABLE_TLS);
    if let Err(err) = disable_tls {
        error!("Failed to disabled tls: {:?}", err);
    }

    let ctx = conn.connect_client(Some("127.0.0.1"), Some(5222), connection_handler);
    match ctx {
        Ok(mut ctx) => {
            ctx.run();
        }
        Err(err) => {
            error!("Something went wrong: {:?}", err);
        }
    }
}
