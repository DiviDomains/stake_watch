use async_trait::async_trait;
use tokio::sync::mpsc;

pub mod polling;
pub mod socketio;

/// Trait for block monitors that detect new blocks and send their hashes
/// through a channel for processing.
#[async_trait]
pub trait BlockMonitor: Send + Sync {
    /// Start monitoring for new blocks. Discovered block hashes are sent
    /// through `tx`. Implementations should spawn background tasks and
    /// return immediately.
    async fn start(&mut self, tx: mpsc::Sender<String>) -> anyhow::Result<()>;

    /// Gracefully stop the monitor and clean up resources.
    async fn stop(&mut self) -> anyhow::Result<()>;
}

/// Create the appropriate monitor based on configuration.
pub fn create_monitor(
    config: &crate::config::BackendConfig,
    rpc: std::sync::Arc<dyn crate::rpc::RpcClient>,
) -> Box<dyn BlockMonitor> {
    match config.backend_type {
        crate::config::BackendType::SocketIo => {
            let sio_config = config
                .socketio
                .as_ref()
                .expect("socketio config required for socketio backend");
            Box::new(socketio::SocketIoMonitor::new(sio_config.clone()))
        }
        crate::config::BackendType::Polling => {
            let poll_config = config
                .polling
                .clone()
                .unwrap_or(crate::config::PollingConfig { interval_secs: 30 });
            Box::new(polling::PollingMonitor::new(poll_config, rpc))
        }
    }
}
