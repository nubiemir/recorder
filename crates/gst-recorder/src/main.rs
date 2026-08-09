//! Recorder binary: an HTTP control surface in front of the XMPP client.
//!
//! One process holds a single XMPP connection (run on its own thread) and any
//! number of meetings. `GET /?room=<name>` makes the recorder join a room;
//! recording then proceeds on its own and stops when the meeting ends.

use config::{Config, ConfigError};
use gst_meet::{config::ConfigSettings, room_manager::RoomManager, xmpp::App};
use libstrophe::Stanza;
use log::{error, info};
use std::{
    env,
    process::exit,
    sync::{Arc, mpsc::channel},
    thread,
};
use tiny_http::{Request, Response, Server};

/// Loads config, starts GStreamer and the XMPP thread, then serves join
/// requests until the listener stops.
///
/// Each request is handled on its own thread and answers as soon as the join
/// presence is queued — not when the room is actually recording.
fn main() {
    let config = init_config().expect("failed to initialize config");
    let config = Arc::new(config);

    gstreamer::init().expect("failed to initialize gstreamer");

    let ip = &config.server.ip;
    let port = &config.server.port;
    let server = Server::http(&format!("{ip}:{port}"));

    match server {
        Ok(server) => {
            info!("started listening on: {:?}", server.server_addr());
            let (tx, rx) = channel::<Stanza>();
            let room_manager = RoomManager::new();

            let app_config = config.clone();
            let tx_for_app = tx.clone();

            let xmpp_handle = thread::spawn(move || {
                match App::connect(&app_config, room_manager, tx_for_app, rx) {
                    Ok(mut app) => app.xmpp_run(),
                    Err(err) => error!("failed connecting to xmpp: {:?}", err),
                }
            });

            for request in server.incoming_requests() {
                let request_config = config.clone();
                let tx = tx.clone();

                thread::spawn(move || {
                    let room = parse_room(&request, &request_config);
                    match App::handle_join_room(&tx, &room) {
                        Ok(room_name) => {
                            info!("sent presence for: {room_name} room");

                            // Explicitly build a structured 200 OK response
                            let message = format!("successfully joined room: {}", room_name);
                            let response = Response::from_string(message).with_status_code(200); // Forces standard HTTP compliance

                            let _ = request.respond(response);
                        }
                        Err(err) => {
                            error!("failed to send presence for: {err:?} room");
                            let response = Response::from_string(format!("Error: {:?}", err))
                                .with_status_code(500);
                            let _ = request.respond(response);
                        }
                    }
                });
            }

            if let Err(err) = xmpp_handle.join() {
                error!("{err:?}");
                exit(1);
            }
        }
        Err(err) => {
            error!("error starting server: {:?}", err);
        }
    }
}

/// Loads `config/default.toml`, overlays `config/<RUN_MODE>.toml` and any
/// `APP_*` environment variables, and installs the logger.
///
/// Paths are relative to the workspace root, so the binary must be run from
/// there.
fn init_config() -> Result<ConfigSettings, ConfigError> {
    let run_mode = env::var("RUN_MODE").unwrap_or_else(|_| "development".into());

    let config = Config::builder()
        .add_source(config::File::with_name(
            "crates/gst-recorder/config/default",
        ))
        .add_source(
            config::File::with_name(&format!("crates/gst-recorder/config/{run_mode}"))
                .required(false),
        )
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let settings = ConfigSettings::new(config)?;

    settings.logger_init();

    Ok(settings)
}

/// Pulls the room name out of the `room=` query parameter, falling back to
/// `"unknown_room"`.
fn parse_room(request: &Request, _config: &Arc<ConfigSettings>) -> String {
    let url = request.url();

    if let Some(pos) = url.find("room=") {
        let query_value = &url[pos + 5..];
        let room_name = query_value.split('&').next().unwrap_or(query_value);
        return room_name.to_string();
    }

    "unknown_room".to_string()
}
