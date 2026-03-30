use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

use crate::config::PollingConfig;
use crate::rpc::RpcClient;

use super::BlockMonitor;

/// Monitor that polls the RPC node at a configured interval to detect
/// new blocks. Suitable for chainz.cryptoid.info and custom nodes
/// that lack a Socket.IO / ZMQ relay.
pub struct PollingMonitor {
    config: PollingConfig,
    rpc: Arc<dyn RpcClient>,
    last_seen_height: Option<u64>,
    task_handle: Option<JoinHandle<()>>,
    /// Signal to stop the polling loop.
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl PollingMonitor {
    pub fn new(config: PollingConfig, rpc: Arc<dyn RpcClient>) -> Self {
        Self {
            config,
            rpc,
            last_seen_height: None,
            task_handle: None,
            shutdown_tx: None,
        }
    }
}

#[async_trait]
impl BlockMonitor for PollingMonitor {
    async fn start(&mut self, tx: mpsc::Sender<String>) -> anyhow::Result<()> {
        let interval_secs = self.config.interval_secs;
        let rpc = Arc::clone(&self.rpc);

        info!(
            interval_secs = interval_secs,
            "Starting polling block monitor"
        );

        // Fetch current height to initialize. On first run, we start from
        // the current tip so we don't replay the entire chain.
        let current_height = rpc.get_block_count().await?;
        self.last_seen_height = Some(current_height);

        info!(
            height = current_height,
            "Polling monitor initialized at current chain tip"
        );

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let mut last_seen = current_height;

        let handle = tokio::spawn(async move {
            let mut poll_interval = interval(Duration::from_secs(interval_secs));

            // Consume the first immediate tick
            poll_interval.tick().await;

            loop {
                tokio::select! {
                    _ = poll_interval.tick() => {
                        // Poll for new blocks
                        let current = match rpc.get_block_count().await {
                            Ok(h) => h,
                            Err(e) => {
                                warn!(error = %e, "Failed to get block count, will retry");
                                continue;
                            }
                        };

                        if current <= last_seen {
                            debug!(
                                current = current,
                                last_seen = last_seen,
                                "No new blocks"
                            );
                            continue;
                        }

                        let new_blocks = current - last_seen;
                        info!(
                            new_blocks = new_blocks,
                            from = last_seen + 1,
                            to = current,
                            "New blocks detected"
                        );

                        if new_blocks > 100 {
                            warn!(
                                gap = new_blocks,
                                "Large block gap detected, processing may take a while"
                            );
                        }

                        // Fetch and send each new block hash in order
                        for height in (last_seen + 1)..=current {
                            match rpc.get_block_hash(height).await {
                                Ok(hash) => {
                                    debug!(
                                        height = height,
                                        hash = %hash,
                                        "Fetched block hash"
                                    );

                                    if let Err(e) = tx.send(hash).await {
                                        error!(
                                            error = %e,
                                            "Block hash channel closed, stopping poller"
                                        );
                                        return;
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        height = height,
                                        error = %e,
                                        "Failed to get block hash, skipping block"
                                    );
                                    // Don't update last_seen past this point
                                    // so we retry on next poll
                                    break;
                                }
                            }

                            last_seen = height;
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Polling monitor received shutdown signal");
                            break;
                        }
                    }
                }
            }

            info!("Polling monitor loop ended");
        });

        self.task_handle = Some(handle);

        Ok(())
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        info!("Stopping polling block monitor");

        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(true);
        }

        if let Some(handle) = self.task_handle.take() {
            // Give the task a reasonable time to shut down
            match tokio::time::timeout(Duration::from_secs(10), handle).await {
                Ok(Ok(())) => {
                    info!("Polling monitor stopped cleanly");
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "Polling monitor task panicked");
                }
                Err(_) => {
                    warn!("Polling monitor stop timed out, task may still be running");
                }
            }
        }

        Ok(())
    }
}
