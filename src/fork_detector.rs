use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, error, info, warn};

use crate::config::ForkDetectionConfig;
use crate::db::{self, DbPool};
use crate::notifier::Notifier;
use crate::rpc::{ChainzClient, JsonRpcClient, RpcClient};

// ---------------------------------------------------------------------------
// ForkDetector
// ---------------------------------------------------------------------------

/// Periodically queries multiple RPC endpoints and compares block hashes at
/// the same height to detect chain forks. When a disagreement is found, it
/// records the event in the database and notifies all fork watchers and
/// admin users.
pub struct ForkDetector {
    db: DbPool,
    notifier: Arc<Notifier>,
    config: ForkDetectionConfig,
    admin_ids: Vec<i64>,
}

/// An endpoint to query, with its name and RPC URL.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub name: String,
    pub rpc_url: String,
}

/// Blocks below the lowest tip at which endpoints are compared. Two nodes can
/// hold different tip blocks for a moment without being on different chains.
pub const CONFIRMATIONS: u64 = 3;

/// One endpoint's answer: its tip, and its block hash at the compared height.
#[derive(Debug, Clone)]
pub struct EndpointView {
    pub name: String,
    pub tip: Option<u64>,
    pub hash: Option<String>,
}

/// The result of asking every endpoint about the same block.
#[derive(Debug, Clone)]
pub struct Comparison {
    /// The height compared.
    pub height: u64,
    pub views: Vec<EndpointView>,
}

impl Comparison {
    /// Whether every endpoint that answered has the same block.
    pub fn agree(&self) -> bool {
        let mut hashes = self.views.iter().filter_map(|v| v.hash.as_deref());
        let first = hashes.next();
        hashes.all(|h| Some(h) == first)
    }
}

/// A client for a fork endpoint: chainz's explorer API, or a node's JSON-RPC.
pub fn endpoint_client(rpc_url: &str) -> Box<dyn RpcClient> {
    if rpc_url.contains("chainz.cryptoid.info") {
        Box::new(ChainzClient::new(rpc_url.to_string(), None))
    } else {
        Box::new(JsonRpcClient::new(rpc_url.to_string(), None, None))
    }
}

/// Every endpoint from the config and the database, without duplicate names.
pub fn gather_endpoints(config: &ForkDetectionConfig, db: &DbPool) -> Result<Vec<Endpoint>> {
    let mut endpoints: Vec<Endpoint> = config
        .endpoints
        .iter()
        .map(|ep| Endpoint {
            name: ep.name.clone(),
            rpc_url: ep.rpc_url.clone(),
        })
        .collect();
    for ep in db::get_fork_endpoints(db)? {
        endpoints.push(Endpoint {
            name: ep.name,
            rpc_url: ep.rpc_url,
        });
    }
    let mut seen = std::collections::HashSet::new();
    endpoints.retain(|ep| seen.insert(ep.name.clone()));
    Ok(endpoints)
}

/// Ask each endpoint for its tip, then for its block hash a few blocks below
/// the lowest tip. None when fewer than two endpoints answered.
pub async fn compare(endpoints: &[Endpoint]) -> Option<Comparison> {
    let clients: Vec<_> = endpoints
        .iter()
        .map(|ep| endpoint_client(&ep.rpc_url))
        .collect();
    let mut tips = Vec::new();
    for (ep, client) in endpoints.iter().zip(&clients) {
        let tip = match client.get_block_count().await {
            Ok(h) => Some(h),
            Err(e) => {
                warn!(endpoint = %ep.name, error = %e, "Fork detection: endpoint unreachable");
                None
            }
        };
        tips.push(tip);
    }
    if tips.iter().flatten().count() < 2 {
        return None;
    }
    let height = tips.iter().flatten().min()?.saturating_sub(CONFIRMATIONS);

    let mut views = Vec::new();
    for ((ep, client), tip) in endpoints.iter().zip(&clients).zip(tips) {
        let hash = match tip {
            Some(_) => match client.get_block_hash(height).await {
                Ok(h) => Some(h),
                Err(e) => {
                    warn!(endpoint = %ep.name, height, error = %e, "Failed to get block hash for fork comparison");
                    None
                }
            },
            None => None,
        };
        views.push(EndpointView {
            name: ep.name.clone(),
            tip,
            hash,
        });
    }
    Some(Comparison { height, views })
}

impl ForkDetector {
    pub fn new(
        db: DbPool,
        notifier: Arc<Notifier>,
        config: ForkDetectionConfig,
        admin_ids: Vec<i64>,
    ) -> Self {
        Self {
            db,
            notifier,
            config,
            admin_ids,
        }
    }

    /// Run the fork detection loop. Only runs if `config.enabled` is true.
    /// Checks every `check_interval_secs` and compares block hashes across
    /// all configured endpoints.
    pub async fn run(&self) {
        if !self.config.enabled {
            info!("Fork detection is disabled");
            return;
        }

        let interval = tokio::time::Duration::from_secs(self.config.check_interval_secs);
        info!(
            interval_secs = self.config.check_interval_secs,
            "Starting fork detection loop"
        );

        loop {
            if let Err(e) = self.check_for_forks().await {
                error!(error = %e, "Fork detection check failed");
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Single iteration of the fork detection check.
    async fn check_for_forks(&self) -> Result<()> {
        let endpoints = gather_endpoints(&self.config, &self.db)?;

        if endpoints.len() < 2 {
            debug!(
                endpoint_count = endpoints.len(),
                "Need at least 2 endpoints for fork detection, skipping"
            );
            return Ok(());
        }

        let Some(cmp) = compare(&endpoints).await else {
            debug!("Fewer than 2 reachable endpoints, skipping comparison");
            return Ok(());
        };
        let min_height = cmp.height;
        let hashes: HashMap<String, String> = cmp
            .views
            .into_iter()
            .filter_map(|v| Some((v.name, v.hash?)))
            .collect();

        if hashes.len() < 2 {
            debug!("Fewer than 2 hash responses, skipping comparison");
            return Ok(());
        }

        // Compare all pairs for mismatches
        let names: Vec<&String> = hashes.keys().collect();
        let mut mismatches: Vec<(String, String, String, String)> = Vec::new();

        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                let name_a = names[i];
                let name_b = names[j];
                let hash_a = &hashes[name_a];
                let hash_b = &hashes[name_b];

                if hash_a != hash_b {
                    mismatches.push((
                        name_a.clone(),
                        hash_a.clone(),
                        name_b.clone(),
                        hash_b.clone(),
                    ));
                }
            }
        }

        if mismatches.is_empty() {
            debug!(
                height = min_height,
                endpoint_count = hashes.len(),
                "No fork detected -- all endpoints agree"
            );
            return Ok(());
        }

        // FORK DETECTED
        warn!(
            height = min_height,
            mismatch_count = mismatches.len(),
            "FORK DETECTED"
        );

        // Record each mismatch in the database
        for (ref ep_a, ref hash_a, ref ep_b, ref hash_b) in &mismatches {
            if let Err(e) = db::record_fork_event(&self.db, min_height, ep_a, hash_a, ep_b, hash_b)
            {
                error!(error = %e, "Failed to record fork event");
            }
        }

        // Build notification message
        let message = self.notifier.format_fork_alert(min_height, &mismatches);

        // Notify all fork watchers
        let fork_watchers = db::get_fork_watchers(&self.db).unwrap_or_default();

        // Merge fork watchers and admin IDs, deduplicating
        let mut notify_ids: Vec<i64> = fork_watchers;
        for &admin_id in &self.admin_ids {
            if !notify_ids.contains(&admin_id) {
                notify_ids.push(admin_id);
            }
        }

        if notify_ids.is_empty() {
            info!("Fork detected but no users subscribed for notifications");
            return Ok(());
        }

        info!(
            height = min_height,
            notify_count = notify_ids.len(),
            "Sending fork detection alerts"
        );

        if let Err(e) = self.notifier.notify_users(&notify_ids, &message).await {
            error!(error = %e, "Failed to send fork detection notifications");
        }

        Ok(())
    }
}
