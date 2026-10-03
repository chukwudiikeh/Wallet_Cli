use bitcoincore_rpc::json::ScanTxOutRequest;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use indicatif::ProgressBar;
use std::collections::HashSet;

use wallet_lib::backend::{ChainBackend, Utxo};
use wallet_lib::bitcoin::{Address, OutPoint, Transaction, Txid};
use wallet_lib::error::{Error, Result};

/// Talks to a real Bitcoin Core node over RPC — in our case, Polar's
/// regtest node. Confirmed UTXOs come from `scantxoutset`, a full,
/// on-demand scan of the chain that needs no wallet. Unconfirmed UTXOs
/// come from scanning the mempool directly, since `scantxoutset` can't
/// see transactions that haven't confirmed yet.
///
/// While syncing it reports how far along it is through `progress`; the
/// bar is hidden, since the TUI draws it itself.
pub struct CoreRpcBackend {
    client: Client,
    progress: ProgressBar,
}

impl CoreRpcBackend {
    pub fn new(url: &str, user: &str, pass: &str, progress: ProgressBar) -> Result<Self> {
        let client = Client::new(url, Auth::UserPass(user.to_string(), pass.to_string()))
            .map_err(|e| Error::Backend(e.to_string()))?;

        Ok(CoreRpcBackend { client, progress })
    }

    /// How many confirmations output `vout` of `txid` has: 0 while it sits in
    /// the mempool, `None` if the node doesn't know it (or it's been spent).
    /// Uses `gettxout`, so it needs no transaction index.
    pub fn tx_confirmations(&self, txid: &Txid, vout: u32) -> Result<Option<u32>> {
        let out = self
            .client
            .get_tx_out(txid, vout, Some(true))
            .map_err(|e| Error::Backend(e.to_string()))?;
        Ok(out.map(|o| o.confirmations))
    }

    fn confirmed_utxos(&self, addresses: &[Address]) -> Result<Vec<Utxo>> {
        let descriptors: Vec<ScanTxOutRequest> = addresses
            .iter()
            .map(|a| ScanTxOutRequest::Single(format!("addr({a})")))
            .collect();

        let result = self
            .client
            .scan_tx_out_set_blocking(&descriptors)
            .map_err(|e| Error::Backend(e.to_string()))?;

        let mut utxos = Vec::new();
        for u in result.unspents {
            let address = addresses
                .iter()
                .find(|a| a.script_pubkey() == u.script_pub_key)
                .cloned()
                .ok_or_else(|| Error::Backend("UTXO for unknown address".to_string()))?;

            utxos.push(Utxo {
                outpoint: OutPoint::new(u.txid, u.vout),
                address,
                amount: u.amount,
                confirmed: true,
            });
        }

        Ok(utxos)
    }

    /// Our unconfirmed outputs in the mempool, plus every outpoint that
    /// mempool transactions spend. `scantxoutset` can't see those spends, so
    /// the caller has to drop them from the confirmed coins itself.
    fn unconfirmed_utxos(
        &self,
        addresses: &[Address],
        mempool_txids: Vec<Txid>,
    ) -> Result<(Vec<Utxo>, HashSet<OutPoint>)> {
        let mut utxos = Vec::new();
        let mut spent = HashSet::new();
        for txid in mempool_txids {
            self.progress.inc(1);
            let tx: Transaction = match self.client.get_raw_transaction(&txid, None) {
                Ok(tx) => tx,
                // It may have left the mempool between listing and
                // fetching — just skip it rather than fail the whole sync.
                Err(_) => continue,
            };

            spent.extend(tx.input.iter().map(|input| input.previous_output));

            for (vout, output) in tx.output.iter().enumerate() {
                if let Some(address) = addresses
                    .iter()
                    .find(|a| a.script_pubkey() == output.script_pubkey)
                {
                    utxos.push(Utxo {
                        outpoint: OutPoint::new(txid, vout as u32),
                        address: address.clone(),
                        amount: output.value,
                        confirmed: false,
                    });
                }
            }
        }

        Ok((utxos, spent))
    }
}

impl ChainBackend for CoreRpcBackend {
    fn utxos_for(&self, addresses: &[Address]) -> Result<Vec<Utxo>> {
        self.progress.set_message("Listing mempool");
        let mempool_txids = self
            .client
            .get_raw_mempool()
            .map_err(|e| Error::Backend(e.to_string()))?;

        // One step for the UTXO-set scan, then one per mempool transaction.
        self.progress.set_length(1 + mempool_txids.len() as u64);
        self.progress.set_position(0);

        self.progress.set_message("Scanning UTXO set");
        let mut utxos = self.confirmed_utxos(addresses)?;
        self.progress.inc(1);

        self.progress.set_message("Checking mempool");
        let (unconfirmed, spent_in_mempool) = self.unconfirmed_utxos(addresses, mempool_txids)?;
        utxos.extend(unconfirmed);

        // Coins a pending transaction already spends aren't spendable again.
        utxos.retain(|u| !spent_in_mempool.contains(&u.outpoint));
        Ok(utxos)
    }

    fn has_history(&self, address: &Address) -> Result<bool> {
        let utxos = self.utxos_for(std::slice::from_ref(address))?;
        Ok(!utxos.is_empty())
    }

    fn broadcast(&self, tx: &Transaction) -> Result<()> {
        self.client.send_raw_transaction(tx).map_err(rpc_error)?;
        Ok(())
    }
}
/// Turn an RPC failure into a backend error, keeping just the node's own
/// message when it sent one (for example "dust" or "missing inputs").
fn rpc_error(e: bitcoincore_rpc::Error) -> Error {
    match e {
        bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(rpc)) => {
            Error::Backend(rpc.message)
        }
        other => Error::Backend(other.to_string()),
    }
}
