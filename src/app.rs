use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use indicatif::ProgressBar;

use crate::backend::CoreRpcBackend;
use crate::send_flow::{SendEvent, SendTracker};
use wallet_lib::backend::Utxo;
use wallet_lib::bitcoin::{Address, Amount};
use wallet_lib::error::Result;
use wallet_lib::wallet::Wallet;

/// How long the finished checklist stays up before returning to the main screen.
const SEND_DONE_LINGER: Duration = Duration::from_millis(2000);

/// How many unused addresses in a row end the scan for a restored wallet's
/// funds (the BIP44 default).
const RECOVERY_GAP_LIMIT: u32 = 20;

/// What a background sync hands back: the wallet it borrowed, and the
/// outcome.
type SyncOutcome = (Wallet<CoreRpcBackend>, Result<()>);

pub enum Screen {
    Welcome,
    ShowMnemonic(String),
    RestoreInput(String),
    Main,
    /// Following a transaction from PSBT to confirmation; see `App::send`.
    Sending,
    Send {
        address: String,
        amount: String,
        editing_address: bool,
    },
}

pub struct RpcConfig {
    pub url: String,
    pub user: String,
    pub pass: String,
}

pub struct App {
    pub screen: Screen,
    pub wallet: Option<Wallet<CoreRpcBackend>>,
    pub address: Option<String>,
    pub balance: (u64, u64),
    /// Snapshot of the wallet's UTXOs as of the last finished sync.
    pub utxos: Vec<Utxo>,
    /// Progress of the sync in flight. Hidden: the UI renders it.
    pub progress: ProgressBar,
    syncing: Option<Receiver<SyncOutcome>>,
    /// Whether the sync in flight is also rebuilding the address list of a
    /// restored wallet.
    recovering: bool,
    /// Kept for the life of the app: on Linux the copied text is only
    /// served while this object is alive.
    clipboard: Option<arboard::Clipboard>,
    /// The send being followed on the `Sending` screen.
    pub send: Option<SendTracker>,
    pub status: String,
    pub status_is_error: bool,
    rpc: RpcConfig,
}

impl App {
    pub fn new(rpc: RpcConfig) -> Self {
        App {
            screen: Screen::Welcome,
            wallet: None,
            address: None,
            balance: (0, 0),
            utxos: Vec::new(),
            progress: ProgressBar::hidden(),
            syncing: None,
            recovering: false,
            clipboard: None,
            send: None,
            status: String::new(),
            status_is_error: false,
            rpc,
        }
    }

    pub fn set_ok(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_is_error = false;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_is_error = true;
    }

    /// Open a fresh connection to the configured node.
    pub fn new_backend(&self) -> Result<CoreRpcBackend> {
        CoreRpcBackend::new(
            &self.rpc.url,
            &self.rpc.user,
            &self.rpc.pass,
            self.progress.clone(),
        )
    }

    /// Copy `text` to the system clipboard and report the outcome.
    pub fn copy_to_clipboard(&mut self, text: String) {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(e) => {
                    self.set_error(format!("Clipboard error: {e}"));
                    return;
                }
            }
        }

        let result = match self.clipboard.as_mut() {
            Some(clipboard) => clipboard.set_text(text),
            None => return,
        };
        match result {
            Ok(()) => self.set_ok("Copied to clipboard."),
            Err(e) => self.set_error(format!("Clipboard error: {e}")),
        }
    }

    pub fn is_syncing(&self) -> bool {
        self.syncing.is_some()
    }

    /// Sync the wallet on a background thread so the UI keeps drawing.
    /// The wallet is moved into the thread and handed back by `poll_sync`.
    pub fn start_sync(&mut self) {
        self.begin_sync(false);
    }

    /// Like `start_sync`, but first scans the chain for the addresses a
    /// restored wallet had already used, so earlier funds show up.
    pub fn start_recovery(&mut self) {
        self.begin_sync(true);
    }

    fn begin_sync(&mut self, recover: bool) {
        let Some(mut wallet) = self.wallet.take() else {
            return;
        };

        self.progress.reset();
        self.progress.set_length(1);
        self.progress.set_message("Connecting");

        self.recovering = recover;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = if recover {
                wallet.recover(RECOVERY_GAP_LIMIT).and_then(|()| wallet.sync())
            } else {
                wallet.sync()
            };
            let _ = tx.send((wallet, result));
        });
        self.syncing = Some(rx);
    }

    /// Check whether the background sync has finished, and if so take the
    /// wallet back and show the result. Call this every tick.
    pub fn poll_sync(&mut self) {
        let Some(rx) = &self.syncing else {
            return;
        };

        match rx.try_recv() {
            Ok((mut wallet, result)) => {
                self.syncing = None;
                self.progress.finish();

                // A restored wallet has no receive address to show until
                // the scan has told it where the used ones end.
                if std::mem::take(&mut self.recovering) {
                    if let Ok(address) = wallet.new_address() {
                        self.address = Some(address.to_string());
                    }
                }
                match result {
                    Ok(()) => {
                        let (confirmed, unconfirmed) = wallet.balance();
                        self.balance = (confirmed.to_sat(), unconfirmed.to_sat());
                        self.utxos = wallet.list_utxos().to_vec();
                        self.set_ok("Synced.");
                    }
                    Err(e) => self.set_error(format!("Sync error: {e}")),
                }
                self.wallet = Some(wallet);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // The sync thread panicked and took the wallet with it.
                self.syncing = None;
                self.recovering = false;
                self.progress.finish();
                self.screen = Screen::Welcome;
                self.set_error("Sync crashed. Restore your wallet from its recovery phrase.");
            }
        }
    }

    /// Start a send on a background thread and switch to the `Sending`
    /// screen to follow it. `form` is the text typed into the send form.
    pub fn start_send(&mut self, recipient: Address, amount: Amount, fee_rate: f32, form: (String, String)) {
        let Some(wallet) = self.wallet.take() else {
            return;
        };

        let watcher = CoreRpcBackend::new(&self.rpc.url, &self.rpc.user, &self.rpc.pass, ProgressBar::hidden());
        match watcher {
            Ok(watcher) => {
                self.send = Some(SendTracker::start(wallet, watcher, recipient, amount, fee_rate, form));
                self.screen = Screen::Sending;
            }
            Err(e) => {
                self.wallet = Some(wallet);
                self.set_error(format!("RPC error: {e}"));
            }
        }
    }

    /// Leave the `Sending` screen after a send that didn't fail: back to the
    /// main screen, and re-sync so the balance reflects the new transaction.
    pub fn finish_send(&mut self) {
        self.send = None;
        self.screen = Screen::Main;
        self.set_ok("Sent!");
        self.start_sync();
    }

    /// Fold in whatever the send worker has reported. Call this every tick.
    pub fn poll_send(&mut self) {
        let Some(send) = &mut self.send else {
            return;
        };

        for event in send.drain() {
            match event {
                SendEvent::WalletBack(wallet) => {
                    // The send re-synced the wallet, so refresh what's shown.
                    let (confirmed, unconfirmed) = wallet.balance();
                    self.balance = (confirmed.to_sat(), unconfirmed.to_sat());
                    self.utxos = wallet.list_utxos().to_vec();
                    self.wallet = Some(wallet);
                }
                other => send.apply(other),
            }
        }

        // Confirmed: leave the finished checklist up a moment, then go home.
        if send.finished_at.is_some_and(|t| t.elapsed() >= SEND_DONE_LINGER) {
            self.finish_send();
            return;
        }

        // The worker died holding the wallet: nothing left to do but restore.
        if self.wallet.is_none() && send.error.is_some() {
            self.send = None;
            self.screen = Screen::Welcome;
            self.set_error("Send crashed. Restore your wallet from its recovery phrase.");
        }
    }
}
