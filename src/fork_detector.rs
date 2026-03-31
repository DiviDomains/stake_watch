use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, error, info, warn};

use crate::config::ForkDetectionConfig;
use crate::db::{self, DbPool};
use crate::notifier::Notifier;
use crate::rpc::JsonRpcClient;
use crate::rpc::RpcClient;

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
struct Endpoint {
    name: String,
    rpc_url: String,
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

    /// Gather all endpoints from config and database, deduplicated by name.
    fn gather_endpoints(&self) -> Result<Vec<Endpoint>> {
        let mut endpoints: Vec<Endpoint> = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        // Config endpoints first
        for ep in &self.config.endpoints {
            if seen_names.insert(ep.name.clone()) {
                endpoints.push(Endpoint {
                    name: ep.name.clone(),
                    rpc_url: ep.rpc_url.clone(),
                });
            }
        }

        // Then DB endpoints
        let db_endpoints = db::get_fork_endpoints(&self.db)?;
        for ep in db_endpoints {
            if seen_names.insert(ep.name.clone()) {
                endpoints.push(Endpoint {
                    name: ep.name,
                    rpc_url: ep.rpc_url,
                });
            }
        }

        Ok(endpoints)
    }

    /// Single iteration of the fork detection check.
    async fn check_for_forks(&self) -> Result<()> {
        let endpoints = self.gather_endpoints()?;

        if endpoints.len() < 2 {
            debug!(
                endpoint_count = endpoints.len(),
                "Need at least 2 endpoints for fork detection, skipping"
            );
            return Ok(());
        }

        // Query each endpoint for its current block count.
        // Collect results, skipping unreachable endpoints.
        let mut heights: HashMap<String, u64> = HashMap::new();
        let mut reachable_endpoints: Vec<&Endpoint> = Vec::new();

        for ep in &endpoints {
            let client = JsonRpcClient::new(ep.rpc_url.clone(), None, None);
            match client.get_block_count().await {
                Ok(height) => {
                    heights.insert(ep.name.clone(), height);
                    reachable_endpoints.push(ep);
                    debug!(endpoint = %ep.name, height, "Endpoint responded");
                }
                Err(e) => {
                    warn!(
                        endpoint = %ep.name,
                        rpc_url = %ep.rpc_url,
                        error = %e,
                        "Fork detection: endpoint unreachable"
                    );
                }
            }
        }

        if reachable_endpoints.len() < 2 {
            debug!("Fewer than 2 reachable endpoints, skipping comparison");
            return Ok(());
        }

        // Find the minimum height across all reachable endpoints so we
        // compare hashes at a height that all endpoints should have.
        let min_height = *heights.values().min().unwrap();

        // Query block hash at min_height from each endpoint
        let mut hashes: HashMap<String, String> = HashMap::new();

        for ep in &reachable_endpoints {
            let client = JsonRpcClient::new(ep.rpc_url.clone(), None, None);
            match client.get_block_hash(min_height).await {
                Ok(hash) => {
                    hashes.insert(ep.name.clone(), hash);
                }
                Err(e) => {
                    warn!(
                        endpoint = %ep.name,
                        height = min_height,
                        error = %e,
                        "Failed to get block hash for fork comparison"
                    );
                }
            }
        }

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
