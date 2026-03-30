use async_trait::async_trait;
use futures_util::FutureExt;
use rust_socketio::{
    asynchronous::{Client, ClientBuilder},
    Payload, TransportType,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::config::SocketIoConfig;
use crate::utils::reverse_hex;

use super::BlockMonitor;

/// JSON structure of ZMQ events received via Socket.IO from the
/// services.divi.domains relay.
#[derive(Debug, Deserialize)]
struct ZmqEvent {
    network: String,
    topic: String,
    data: String,
    #[allow(dead_code)]
    timestamp: Option<f64>,
}

/// Monitor that connects to a Socket.IO ZMQ relay to receive real-time
/// block hash notifications.
pub struct SocketIoMonitor {
    config: SocketIoConfig,
    client: Option<Client>,
    /// Tracks the last height we processed for gap detection.
    last_processed_height: Arc<Mutex<Option<u64>>>,
}

impl SocketIoMonitor {
    pub fn new(config: SocketIoConfig) -> Self {
        Self {
            config,
            client: None,
            last_processed_height: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl BlockMonitor for SocketIoMonitor {
    async fn start(&mut self, tx: mpsc::Sender<String>) -> anyhow::Result<()> {
        let network_filter = self.config.network_filter.clone();
        let last_height = Arc::clone(&self.last_processed_height);

        let url = self.config.url.clone();
        let path = self.config.path.clone();

        info!(
            url = %url,
            path = %path,
            network_filter = %network_filter,
            "Starting Socket.IO block monitor"
        );

        let tx_clone = tx.clone();
        let network_filter_clone = network_filter.clone();

        // Handler for "zmq-event" messages.
        // rust_socketio v0.6 requires callbacks to return BoxFuture<'static, ()>.
        let zmq_handler = move |payload: Payload, _client: Client| {
            let tx = tx_clone.clone();
            let network_filter = network_filter_clone.clone();
            let last_height = Arc::clone(&last_height);

            async move {
                let json_str = match &payload {
                    Payload::Text(values) => {
                        if let Some(val) = values.first() {
                            val.to_string()
                        } else {
                            warn!("Received zmq-event with empty text payload");
                            return;
                        }
                    }
                    Payload::Binary(bytes) => {
                        match String::from_utf8(bytes.to_vec()) {
                            Ok(s) => s,
                            Err(e) => {
                                warn!("Failed to decode binary zmq-event payload: {}", e);
                                return;
                            }
                        }
                    }
                    #[allow(deprecated)]
                    Payload::String(s) => s.clone(),
                };

                let event: ZmqEvent = match serde_json::from_str(&json_str) {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(
                            payload = %json_str,
                            error = %e,
                            "Failed to parse zmq-event JSON"
                        );
                        return;
                    }
                };

                // Filter by network and topic
                if event.network != network_filter {
                    debug!(
                        network = %event.network,
                        expected = %network_filter,
                        "Ignoring zmq-event for different network"
                    );
                    return;
                }

                if event.topic != "hashblock" {
                    debug!(
                        topic = %event.topic,
                        "Ignoring non-hashblock zmq-event"
                    );
                    return;
                }

                // ZMQ delivers block hashes in little-endian byte order.
                // Reverse to get the standard big-endian display format.
                let block_hash = reverse_hex(&event.data);

                info!(
                    block_hash = %block_hash,
                    network = %event.network,
                    "Received new block hash via Socket.IO"
                );

                // Update last processed height tracker
                {
                    let mut height = last_height.lock().await;
                    if height.is_none() {
                        *height = Some(0);
                    }
                }

                if let Err(e) = tx.send(block_hash).await {
                    error!("Failed to send block hash to processor: {}", e);
                }
            }
            .boxed()
        };

        // Connect handler
        let connect_handler = move |_payload: Payload, _client: Client| {
            async move {
                info!("Socket.IO connected to ZMQ relay");
            }
            .boxed()
        };

        // Disconnect handler
        let disconnect_handler = move |_payload: Payload, _client: Client| {
            async move {
                warn!("Socket.IO disconnected from ZMQ relay");
            }
            .boxed()
        };

        // Error handler
        let error_handler = move |payload: Payload, _client: Client| {
            async move {
                match &payload {
                    Payload::Text(values) => {
                        warn!(error = ?values, "Socket.IO error");
                    }
                    _ => {
                        warn!("Socket.IO error (non-text payload)");
                    }
                }
            }
            .boxed()
        };

        // Build and connect the Socket.IO client.
        // The Socket.IO transport path is set via the URL path, not HTTP headers.
        // rust_socketio only defaults to "/socket.io/" when the URL path is "/".
        // Combine base URL and path to produce e.g. "https://host/zmq/socket.io/".
        let connect_url = {
            let base = url.trim_end_matches('/');
            let p = path.trim_start_matches('/').trim_end_matches('/');
            format!("{}/{}/", base, p)
        };

        info!(connect_url = %connect_url, "Connecting Socket.IO client");

        let client = ClientBuilder::new(&connect_url)
            .transport_type(TransportType::Websocket)
            .reconnect_on_disconnect(true)
            .on("zmq-event", zmq_handler)
            .on("open", connect_handler)
            .on("close", disconnect_handler)
            .on("error", error_handler)
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect Socket.IO client: {}", e))?;

        info!("Socket.IO client connected successfully");
        self.client = Some(client);

        Ok(())
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(client) = self.client.take() {
            info!("Disconnecting Socket.IO client");
            client
                .disconnect()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to disconnect Socket.IO client: {}", e))?;
            info!("Socket.IO client disconnected");
        }
        Ok(())
    }
}
