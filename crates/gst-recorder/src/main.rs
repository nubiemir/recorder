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
            let room_manager = Arc::new(Mutex::new(RoomManager::new()));

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

fn parse_room(request: &Request, config: &Arc<ConfigSettings>) -> String {
    let room = request
        .url()
        .trim_start_matches(config.server.start_pattern_trim.as_str());

    room.to_string()
}
