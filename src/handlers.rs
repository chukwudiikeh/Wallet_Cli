use ratatui::crossterm::event::KeyCode;
use std::str::FromStr;

use crate::app::{App, Screen};
use wallet_lib::bitcoin::{Address, Amount, Network};
use wallet_lib::wallet::Wallet;

/// Handle one key press for whichever screen is active.
/// Returns true if the app should quit.
pub fn handle_key(app: &mut App, code: KeyCode) -> bool {
    // The wallet is on the sync thread; ignore input until it's back.
    if app.is_syncing() {
        return false;
    }

    match app.screen {
        Screen::Welcome => handle_welcome(app, code),
        Screen::ShowMnemonic(_) => handle_show_mnemonic(app, code),
        Screen::RestoreInput(_) => handle_restore_input(app, code),
        Screen::Main => handle_main(app, code),
        Screen::Send { .. } => handle_send(app, code),
        Screen::Sending => handle_sending(app, code),
    }
}

fn handle_welcome(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('n') => match app.new_backend() {
            Ok(backend) => match Wallet::generate(Network::Regtest, backend) {
                Ok((wallet, words)) => {
                    app.wallet = Some(wallet);
                    app.screen = Screen::ShowMnemonic(words);
                }
                Err(e) => app.set_error(format!("Error: {e}")),
            },
            Err(e) => app.set_error(format!("RPC error: {e}")),
        },
        KeyCode::Char('r') => app.screen = Screen::RestoreInput(String::new()),
        KeyCode::Char('q') => return true,
        _ => {}
    }
    false
}

fn handle_show_mnemonic(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Enter => {
            let address = app.wallet.as_mut().unwrap().new_address().unwrap();
            app.address = Some(address.to_string());
            app.screen = Screen::Main;
        }
        KeyCode::Char('c') => {
            if let Screen::ShowMnemonic(words) = &app.screen {
                let words = words.clone();
                app.copy_to_clipboard(words);
            }
        }
        _ => {}
    }
    false
}

fn handle_restore_input(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char(c) => {
            if let Screen::RestoreInput(words) = &mut app.screen {
                words.push(c);
            }
        }
        KeyCode::Backspace => {
            if let Screen::RestoreInput(words) = &mut app.screen {
                words.pop();
            }
        }
        KeyCode::Enter => {
            let words = match &app.screen {
                Screen::RestoreInput(w) => w.clone(),
                _ => return false,
            };

            match app.new_backend() {
                Ok(backend) => match Wallet::from_mnemonic(&words, Network::Regtest, backend) {
                    Ok(wallet) => {
                        app.wallet = Some(wallet);
                        app.screen = Screen::Main;
                        // Find the addresses this wallet already used, then
                        // show its balance.
                        app.start_recovery();
                    }
                    Err(e) => app.set_error(format!("Error: {e}")),
                },
                Err(e) => app.set_error(format!("RPC error: {e}")),
            }
        }
        _ => {}
    }
    false
}

fn handle_main(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('a') => {
            let address = app.wallet.as_mut().unwrap().new_address().unwrap();
            app.address = Some(address.to_string());
        }
        KeyCode::Char('c') => {
            if let Some(address) = app.address.clone() {
                app.copy_to_clipboard(address);
            }
        }
        KeyCode::Char('s') => app.start_sync(),
        KeyCode::Char('p') => {
            app.screen = Screen::Send {
                address: String::new(),
                amount: String::new(),
                editing_address: true,
            };
        }
        KeyCode::Char('q') => return true,
        _ => {}
    }
    false
}

fn handle_send(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Tab => {
            if let Screen::Send {
                editing_address, ..
            } = &mut app.screen
            {
                *editing_address = !*editing_address;
            }
        }
        KeyCode::Char(c) => {
            if let Screen::Send {
                address,
                amount,
                editing_address,
            } = &mut app.screen
            {
                if *editing_address {
                    address.push(c);
                } else if c.is_ascii_digit() {
                    amount.push(c);
                }
            }
        }
        KeyCode::Backspace => {
            if let Screen::Send {
                address,
                amount,
                editing_address,
            } = &mut app.screen
            {
                if *editing_address {
                    address.pop();
                } else {
                    amount.pop();
                }
            }
        }
        KeyCode::Esc => app.screen = Screen::Main,
        KeyCode::Enter => {
            let (address_text, amount_text) = match &app.screen {
                Screen::Send {
                    address, amount, ..
                } => (address.clone(), amount.clone()),
                _ => return false,
            };

            let parsed_address = Address::from_str(&address_text)
                .ok()
                .and_then(|a| a.require_network(Network::Regtest).ok());
            let parsed_sats: Option<u64> = amount_text.parse().ok();

            match (parsed_address, parsed_sats) {
                (Some(recipient), Some(sats)) => {
                    // Below this the node refuses the payment as dust.
                    let minimum = recipient.script_pubkey().minimal_non_dust().to_sat();
                    if sats < minimum {
                        app.set_error(format!(
                            "{sats} sats is dust. The smallest payment to this address is {minimum} sats."
                        ));
                    } else {
                        app.start_send(recipient, Amount::from_sat(sats), 2.0, (address_text, amount_text));
                    }
                }
                _ => app.set_error("Invalid address or amount"),
            }
        }
        _ => {}
    }
    false
}

fn handle_sending(app: &mut App, code: KeyCode) -> bool {
    // The wallet is still on the send thread until it's broadcast or failed.
    if app.wallet.is_none() {
        return false;
    }

    if matches!(code, KeyCode::Esc | KeyCode::Enter) {
        let Some(send) = app.send.take() else {
            app.screen = Screen::Main;
            return false;
        };

        match send.error {
            // Failed: back to the form with what was typed, so it can be fixed.
            Some(error) => {
                let (address, amount) = send.form;
                app.screen = Screen::Send {
                    address,
                    amount,
                    editing_address: false,
                };
                app.set_error(error);
            }
            None => app.finish_send(),
        }
    }
    false
}
