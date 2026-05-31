use config::{Config, ConfigError};
use gst_meet::{config::ConfigSettings, room_manager::RoomManager, xmpp::App};
use libstrophe::Stanza;
use log::{error, info};
use std::{
    env,
    process::exit,
    sync::{Arc, Mutex, mpsc::channel},
    thread,
};
use tiny_http::{Request, Response, Server};

fn main() {
    let config = init_config().unwrap();
    let config = Arc::new(ConfigSettings::new(config).unwrap());
    config.logger_init();
    gstreamer::init().unwrap();

    let ip = &config.server.ip;
    let port = &config.server.port;
    let server = Server::http(&format!("{ip}:{port}"));
    match server {
        Ok(server) => {
            info!("started listening on: {:?}", server.server_addr());
            let (tx, rx) = channel::<Stanza>();
            let room_manager = Arc::new(Mutex::new(RoomManager::new()));

            // Clone config values the XMPP thread needs
            let config_for_app = config.clone();
            let tx_for_app = tx.clone();

            // Spawn XMPP thread — App is created and owned entirely here
            let room_manager_for_xmpp = room_manager.clone();
            let xmpp_handle = thread::spawn(move || {
                match App::xmpp_connect(&config_for_app, room_manager_for_xmpp, tx_for_app, rx) {
                    Ok(mut app) => app.xmpp_run(),
                    Err(err) => error!("failed connecting to xmpp: {:?}", err),
                }
            });

            for request in server.incoming_requests() {
                let config = Arc::clone(&config);
                let tx = tx.clone();

                thread::spawn(move || {
                    let room = parse_room(&request, &config);
                    match App::handle_join_room(&tx, &room) {
                        Ok(room_name) => {
                            info!("sent presence for: {room_name} room");
                            let response = Response::from_string(
                                format!("successfully joined room: {}", room_name).to_string(),
                            );
                            let _ = request.respond(response);
                        }
                        Err(err) => {
                            error!("failed to send presence for: {err:?} room");
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

fn init_config() -> Result<Config, ConfigError> {
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

    Ok(config)
}

fn parse_room(request: &Request, config: &Arc<ConfigSettings>) -> String {
    let room = request
        .url()
        .trim_start_matches(config.server.start_pattern_trim.as_str());

    room.to_string()
}
