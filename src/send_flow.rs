//! Runs a send on a background thread and reports each step of the
//! transaction's life as it happens: PSBT built, signed, broadcast, seen in
//! the mempool, then confirmed in a block.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use indicatif::ProgressBar;

use crate::backend::CoreRpcBackend;
use wallet_lib::bitcoin::{Address, Amount, Txid};
use wallet_lib::wallet::Wallet;

pub const STAGES: [&str; 6] = [
    "Check balance",
    "Build PSBT",
    "Sign",
    "Broadcast",
    "Seen in mempool",
    "Confirmed in a block",
];

/// Index of the last stage, the one that waits for a block.
pub const LAST_STAGE: usize = STAGES.len() - 1;

/// The stage reached once a block has confirmed the transaction.
pub const FINISHED: u64 = STAGES.len() as u64;

/// How long to linger on the quick early stages so they can be seen.
pub const STAGE_PAUSE: Duration = Duration::from_millis(800);
/// How often to ask the node whether the transaction has confirmed.
const POLL_EVERY: Duration = Duration::from_secs(2);

pub enum SendEvent {
    /// The stage with this index has started; every earlier one is done.
    Active(usize),
    /// The PSBT is built: a human summary of it, and the transaction id.
    Built { summary: String, txid: Txid },
    /// The wallet is free again and goes back to the app.
    WalletBack(Wallet<CoreRpcBackend>),
    /// One look at the chain: `None` if the node doesn't know the
    /// transaction, otherwise its confirmation count (0 = mempool).
    Poll(Option<u32>),
    Failed(String),
}

pub struct SendTracker {
    rx: Receiver<SendEvent>,
    /// Position is the index of the active stage; hidden, the UI draws it.
    pub bar: ProgressBar,
    pub summary: Option<String>,
    pub txid: Option<Txid>,
    pub confirmations: u32,
    pub error: Option<String>,
    /// The address and amount typed in, so a failed send can go back to them.
    pub form: (String, String),
    /// When the confirmation arrived; the app returns to the main screen
    /// shortly after.
    pub finished_at: Option<Instant>,
    stage_started: Instant,
    done: bool,
}

impl SendTracker {
    /// Move `wallet` onto a thread that builds, signs, broadcasts and then
    /// watches the transaction. `watcher` is a separate connection to the
    /// node for the watching.
    pub fn start(
        wallet: Wallet<CoreRpcBackend>,
        watcher: CoreRpcBackend,
        recipient: Address,
        amount: Amount,
        fee_rate: f32,
        form: (String, String),
    ) -> Self {
        let bar = ProgressBar::hidden();
        bar.set_length(FINISHED);

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            // A send error just means the UI stopped listening.
            let _ = run(wallet, watcher, recipient, amount, fee_rate, &tx);
        });

        SendTracker {
            rx,
            bar,
            summary: None,
            txid: None,
            confirmations: 0,
            error: None,
            form,
            finished_at: None,
            stage_started: Instant::now(),
            done: false,
        }
    }

    /// Everything the worker has reported since the last call. If it died
    /// without finishing, that comes back as a `Failed` event.
    pub fn drain(&mut self) -> Vec<SendEvent> {
        let mut events = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The worker exits right after its last event, so that
                    // event may be in this very batch and not yet applied.
                    let ended = events.iter().any(|e| match e {
                        SendEvent::Failed(_) => true,
                        SendEvent::Poll(Some(confirmations)) => *confirmations >= 1,
                        _ => false,
                    });
                    if !self.done && !ended {
                        events.push(SendEvent::Failed("send worker stopped unexpectedly".into()));
                    }
                    break;
                }
            }
        }
        events
    }

    /// Fold one event (other than `WalletBack`) into what the UI shows.
    pub fn apply(&mut self, event: SendEvent) {
        match event {
            SendEvent::Active(stage) => {
                self.bar.set_position(stage as u64);
                self.stage_started = Instant::now();
            }
            SendEvent::Built { summary, txid } => {
                self.summary = Some(summary);
                self.txid = Some(txid);
            }
            SendEvent::Poll(confirmations) => {
                self.confirmations = confirmations.unwrap_or(0);
                if confirmations.is_some_and(|c| c >= 1) {
                    self.bar.set_position(FINISHED);
                    self.finished_at.get_or_insert_with(Instant::now);
                    self.done = true;
                }
            }
            SendEvent::Failed(message) => {
                self.error = Some(message);
                self.done = true;
            }
            SendEvent::WalletBack(_) => {}
        }
    }

    /// How long the current stage has been running.
    pub fn stage_elapsed(&self) -> Duration {
        self.stage_started.elapsed()
    }

    pub fn is_finished(&self) -> bool {
        self.bar.position() >= FINISHED
    }
}

fn run(
    mut wallet: Wallet<CoreRpcBackend>,
    watcher: CoreRpcBackend,
    recipient: Address,
    amount: Amount,
    fee_rate: f32,
    tx: &Sender<SendEvent>,
) -> Result<(), mpsc::SendError<SendEvent>> {
    // Report a failure, handing the wallet back first.
    let fail = |wallet: Wallet<CoreRpcBackend>, message: String| {
        tx.send(SendEvent::WalletBack(wallet))?;
        tx.send(SendEvent::Failed(message))
    };

    // Start from what the chain says now, not from the last sync: coins
    // spent since then must not be picked again.
    tx.send(SendEvent::Active(0))?;
    thread::sleep(STAGE_PAUSE);
    if let Err(e) = wallet.sync() {
        return fail(wallet, format!("Sync failed: {e}"));
    }
    let (confirmed, unconfirmed) = wallet.balance();
    let available = confirmed + unconfirmed;
    if available < amount {
        let message = format!(
            "Insufficient balance: sending {} sats but only {} sats are spendable",
            amount.to_sat(),
            available.to_sat(),
        );
        return fail(wallet, message);
    }

    tx.send(SendEvent::Active(1))?;
    thread::sleep(STAGE_PAUSE);
    let mut psbt = match wallet.build_tx(&recipient, amount, fee_rate) {
        Ok(psbt) => psbt,
        Err(e) => return fail(wallet, format!("Build failed: {e}")),
    };
    let txid = psbt.unsigned_tx.compute_txid();
    let fee = match psbt.fee() {
        Ok(fee) => format!("{} sats", fee.to_sat()),
        Err(_) => "unknown".to_string(),
    };
    tx.send(SendEvent::Built {
        summary: format!(
            "{} input(s), {} output(s), fee {fee}",
            psbt.unsigned_tx.input.len(),
            psbt.unsigned_tx.output.len(),
        ),
        txid,
    })?;

    tx.send(SendEvent::Active(2))?;
    thread::sleep(STAGE_PAUSE);
    if let Err(e) = wallet.sign(&mut psbt) {
        return fail(wallet, format!("Signing failed: {e}"));
    }

    tx.send(SendEvent::Active(3))?;
    thread::sleep(STAGE_PAUSE);
    if let Err(e) = wallet.broadcast(&psbt.unsigned_tx) {
        return fail(wallet, format!("Broadcast failed: {e}"));
    }
    tx.send(SendEvent::WalletBack(wallet))?;

    // Watch the recipient output (always output 0) until a block has it.
    tx.send(SendEvent::Active(4))?;
    let mut in_mempool = false;
    loop {
        // An RPC hiccup shouldn't end the watch; just look again next time.
        let seen = watcher.tx_confirmations(&txid, 0).ok().flatten();

        if seen.is_some() && !in_mempool {
            in_mempool = true;
            tx.send(SendEvent::Active(5))?;
        }
        // Sent every round, so a UI that has gone away is noticed.
        tx.send(SendEvent::Poll(seen))?;

        if seen.is_some_and(|c| c >= 1) {
            return Ok(());
        }
        thread::sleep(POLL_EVERY);
    }
}
