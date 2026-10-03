# wallet_cli

A terminal Bitcoin wallet built with Rust and [ratatui](https://ratatui.rs). It connects to a Bitcoin Core node over RPC and is designed for a local **regtest** network such as [Polar](https://lightningpolar.com).

`wallet_cli` is the user interface. Key management, coin selection and transaction building live in the companion `wallet_lib` crate.

> **Warning:** This is a learning/development project. Use it on regtest only, never with real funds.

## Features

- **Create a wallet**: generates a mnemonic phrase and shows it once so you can back it up (copy it to the clipboard with `c`).
- **Restore a wallet**: enter an existing mnemonic and the wallet rescans for funds, stopping after 20 consecutive unused addresses (the BIP44 gap limit).
- **Balance and UTXOs**: shows confirmed and unconfirmed balances, synced in the background with a live progress bar.
- **Receive**: displays a receive address, copyable to the system clipboard, and can generate a fresh one on demand.
- **Send**: enter a destination address and amount. A live checklist follows the transaction from PSBT creation through broadcast to confirmation.

## Requirements

- Rust (edition 2024, so a recent stable toolchain)
- A running Bitcoin Core node with RPC enabled, such as a Polar regtest node
- The `wallet_lib` crate checked out next to this project (see below)
- A system clipboard (on Linux, X11 or Wayland) for the copy features

## Project layout

`wallet_cli` expects `wallet_lib` as a sibling directory:

```
Rust/
├── wallet_cli/   # this project
└── wallet_lib/   # path dependency (../wallet_lib)
```

## Setup

1. Start a regtest Bitcoin Core node (for example, create a network in Polar and start it).
2. Create a `.env` file in the project root with the node's RPC details:

   ```env
   RPC_URL=http://127.0.0.1:18443
   RPC_USER=your_rpc_user
   RPC_PASS=your_rpc_password
   ```

   Polar shows the RPC host, port, username and password on each node's **Connect** tab. The `.env` file is git-ignored.

3. Build and run:

   ```sh
   cargo run --release
   ```

The app panics on startup if any of the three variables is missing.

## Usage

| Screen            | Keys                                                         |
| ----------------- | ------------------------------------------------------------ |
| Welcome           | `n` new wallet · `r` restore · `q` quit                      |
| Mnemonic backup   | `c` copy phrase · `Enter` continue                           |
| Restore           | type the phrase · `Enter` restore                            |
| Main              | `c` copy address · `a` new address · `s` sync · `p` send · `q` quit |
| Send              | `Tab` switch field · `Enter` send · `Esc` cancel             |
| Send progress     | `Esc` / `Enter` when finished                                |

`Ctrl+C` quits from any screen.

On regtest, fund the wallet by sending coins to its receive address from another node or the Polar mining tools, mine a block, then press `s` to sync.

## How it works

The wallet uses a custom `CoreRpcBackend` that implements `wallet_lib`'s `ChainBackend` trait. It needs no node-side wallet and no transaction index:

- **Confirmed UTXOs** come from `scantxoutset`, a full on-demand scan of the UTXO set for the wallet's addresses.
- **Unconfirmed UTXOs** come from scanning mempool transactions directly, since `scantxoutset` can't see them.
- **Pending spends**: outputs already spent by mempool transactions are removed from the spendable set.
- **Confirmations** for a sent transaction are read with `gettxout`.
- **Broadcasting** uses `sendrawtransaction`, surfacing the node's own error message (for example `dust` or `missing inputs`).

Syncs and sends run on background threads so the UI stays responsive.

### Source overview

| File            | Purpose                                                  |
| --------------- | -------------------------------------------------------- |
| `main.rs`       | Entry point                                              |
| `terminal.rs`   | Loads config, sets up the terminal, runs the event loop  |
| `app.rs`        | App state, screens, background sync and send orchestration |
| `handlers.rs`   | Per-screen keyboard handling                             |
| `send_flow.rs`  | Tracks a send from PSBT to confirmation                  |
| `backend.rs`    | Bitcoin Core RPC backend                                 |
| `ui.rs`         | All ratatui rendering                                    |

## Dependencies

[`ratatui`](https://crates.io/crates/ratatui), [`bitcoincore-rpc`](https://crates.io/crates/bitcoincore-rpc), [`indicatif`](https://crates.io/crates/indicatif), [`arboard`](https://crates.io/crates/arboard), [`dotenvy`](https://crates.io/crates/dotenvy), and the local `wallet_lib`.
