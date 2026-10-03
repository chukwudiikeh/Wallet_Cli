use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};

use crate::app::{App, RpcConfig};
use crate::handlers;
use crate::ui;

pub fn terminal() {
    dotenvy::dotenv().ok();
    let rpc = RpcConfig {
        url: std::env::var("RPC_URL").expect("RPC_URL not set in .env"),
        user: std::env::var("RPC_USER").expect("RPC_USER not set in .env"),
        pass: std::env::var("RPC_PASS").expect("RPC_PASS not set in .env"),
    };

    let mut app = App::new(rpc);
    let mut terminal = ratatui::init();

    loop {
        app.poll_sync();
        app.poll_send();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();

        // Wake regularly so the sync progress keeps animating.
        if !event::poll(Duration::from_millis(100)).unwrap() {
            continue;
        }

        if let Event::Key(key) = event::read().unwrap() {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                break;
            }

            if handlers::handle_key(&mut app, key.code) {
                break;
            }
        }
    }

    ratatui::restore();
}
