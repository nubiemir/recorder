use std::sync::Arc;

use libstrophe::Connection;

pub struct App {
    pub xmpp_connection: Arc<Connection<'static, 'static>>,
}

impl App {
    pub fn new(connection: Connection<'static, 'static>) -> Self {
        App {
            xmpp_connection: Arc::new(connection),
        }
    }
}
