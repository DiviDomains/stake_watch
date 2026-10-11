use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use anyhow::Result;
use tracing::{debug, error, info, warn};

use crate::config::ForkDetectionConfig;
use crate::db::{self, DbPool};
use crate::notifier::Notifier;
use crate::rpc::{ChainTip, ChainzClient, JsonRpcClient, RpcClient};

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
    /// Per node, the hashes of its last REORG_WINDOW blocks at the last check.
    seen: Mutex<HashMap<String, BTreeMap<u64, String>>>,
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
                // Anything but a block hash (an error page, a quoted string) is no answer, not a
                // fork: it once raised a false alert whose broken link Telegram then rejected.
                Ok(h) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) => Some(h),
                Ok(h) => {
                    warn!(endpoint = %ep.name, height, answer = %h.chars().take(80).collect::<String>(), "Fork detection: not a block hash");
                    None
                }
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

/// Side branches are looked at for this many blocks back (about a day), so
/// old history doesn't raise alerts on a restart.
pub const BRANCH_WINDOW: u64 = 1440;

/// One node's view of its own chain branches.
#[derive(Debug, Clone)]
pub struct BranchView {
    pub name: String,
    /// The main chain's tip height.
    pub active_height: u64,
    pub active_hash: String,
    /// Side branches that ended within the window, longest first.
    pub recent: Vec<ChainTip>,
}

impl BranchView {
    pub fn from_tips(name: &str, tips: &[ChainTip]) -> Option<Self> {
        let active = tips.iter().find(|t| t.status == "active")?;
        let since = active.height.saturating_sub(BRANCH_WINDOW);
        let mut recent: Vec<ChainTip> = tips
            .iter()
            .filter(|t| t.status != "active" && t.height > since)
            .cloned()
            .collect();
        recent.sort_by(|a, b| b.branchlen.cmp(&a.branchlen).then(b.height.cmp(&a.height)));
        Some(Self {
            name: name.to_string(),
            active_height: active.height,
            active_hash: active.hash.clone(),
            recent,
        })
    }

    /// Side branches longer than `max_len` blocks.
    pub fn deep(&self, max_len: u64) -> Vec<&ChainTip> {
        self.recent.iter().filter(|t| t.branchlen > max_len).collect()
    }
}

/// The last block both chains share: where the side branch left the main chain.
pub fn fork_point(tip: &ChainTip) -> u64 {
    tip.height.saturating_sub(tip.branchlen)
}

/// Blocks re-checked at each tick for a reorg: about half an hour of blocks,
/// six times what arrives between two checks.
pub const REORG_WINDOW: u64 = 30;

/// A node replaced blocks it had: `depth` of them, after block `fork_point`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reorg {
    pub fork_point: u64,
    pub depth: u64,
}

/// Compare a node's block hashes now with those at the last check. The lowest
/// height whose hash changed is where the node switched chains; every block it
/// had from there up was replaced. A reorg reaching below the window is
/// reported as the window's depth: at least that deep.
pub fn find_reorg(before: &BTreeMap<u64, String>, now: &BTreeMap<u64, String>) -> Option<Reorg> {
    let lowest = before
        .iter()
        .find(|(h, hash)| now.get(h).is_some_and(|n| n != *hash))
        .map(|(h, _)| *h)?;
    let old_top = *before.keys().next_back()?;
    Some(Reorg { fork_point: lowest - 1, depth: old_top - lowest + 1 })
}

/// What /forkstatus shows of the reorg watch.
#[derive(Debug, Clone)]
pub struct ReorgStatus {
    pub checked_at: DateTime<Utc>,
    pub since: DateTime<Utc>,
    /// The deepest reorg seen since `since`: node, reorg, when.
    pub deepest: Option<(String, Reorg, DateTime<Utc>)>,
}

static REORG_STATUS: Mutex<Option<ReorgStatus>> = Mutex::new(None);

pub fn reorg_status() -> Option<ReorgStatus> {
    REORG_STATUS.lock().ok()?.clone()
}

/// The last branch check: when it ran, and what each node reported. Kept for
/// /forkstatus, which must not ask the nodes itself (each ask freezes them).
static LAST_BRANCHES: Mutex<Option<(DateTime<Utc>, Vec<BranchView>)>> = Mutex::new(None);

pub fn last_branches() -> Option<(DateTime<Utc>, Vec<BranchView>)> {
    LAST_BRANCHES.lock().ok()?.clone()
}

/// Ask every node that can list its chain branches (getchaintips; an explorer
/// API can't) for them.
pub async fn branches(endpoints: &[Endpoint]) -> Vec<BranchView> {
    let mut views = Vec::new();
    for ep in endpoints {
        match endpoint_client(&ep.rpc_url).get_chain_tips().await {
            Ok(Some(tips)) => match BranchView::from_tips(&ep.name, &tips) {
                Some(v) => views.push(v),
                None => warn!(endpoint = %ep.name, "getchaintips: no active tip"),
            },
            Ok(None) => {}
            Err(e) => warn!(endpoint = %ep.name, error = %e, "getchaintips failed"),
        }
    }
    views
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
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Run fork detection, if `config.enabled`: compare the endpoints' block
    /// hashes every `check_interval_secs`, and look for deep side branches
    /// every `branch_check_interval_secs`.
    pub async fn run(&self) {
        if !self.config.enabled {
            info!("Fork detection is disabled");
            return;
        }
        info!(
            interval_secs = self.config.check_interval_secs,
            branch_interval_secs = self.config.branch_check_interval_secs,
            "Starting fork detection loop"
        );
        // Two loops side by side: the branch check waits half a minute for its
        // answer, and the endpoint comparison doesn't wait for it.
        tokio::join!(self.compare_loop(), self.branch_loop());
    }

    async fn compare_loop(&self) {
        let interval = std::time::Duration::from_secs(self.config.check_interval_secs);
        loop {
            match gather_endpoints(&self.config, &self.db) {
                Ok(endpoints) => {
                    self.compare_endpoints(&endpoints).await;
                    self.check_reorgs(&endpoints).await;
                }
                Err(e) => error!(error = %e, "Fork detection check failed"),
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn branch_loop(&self) {
        if self.config.branch_check_interval_secs == 0 {
            info!("Branch check (getchaintips) is off");
            return;
        }
        let interval = std::time::Duration::from_secs(self.config.branch_check_interval_secs);
        loop {
            match gather_endpoints(&self.config, &self.db) {
                Ok(endpoints) => self.check_branches(&endpoints).await,
                Err(e) => error!(error = %e, "Branch check failed"),
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// A side branch longer than `max_branch_len` on any node: part of the
    /// network followed another chain for that many blocks. Alerted once per
    /// fork, however long the branch grows.
    async fn check_branches(&self, endpoints: &[Endpoint]) {
        let views = branches(endpoints).await;
        for v in &views {
            info!(
                endpoint = %v.name,
                tip = v.active_height,
                side_branches = v.recent.len(),
                longest = v.recent.first().map_or(0, |t| t.branchlen),
                "Chain branches checked"
            );
        }
        if let Ok(mut last) = LAST_BRANCHES.lock() {
            *last = Some((Utc::now(), views.clone()));
        }
        for view in views {
            for tip in view.deep(self.config.max_branch_len) {
                let point = fork_point(tip);
                let side = format!("{}: {}-block side branch", view.name, tip.branchlen);
                let main = format!("{}: main chain", view.name);
                match db::fork_event_recorded(&self.db, point, &format!("{}: %side branch", view.name)) {
                    Ok(false) => {}
                    Ok(true) => continue,
                    Err(e) => {
                        error!(error = %e, "Failed to look up fork events");
                        continue;
                    }
                }
                warn!(
                    endpoint = %view.name,
                    fork_point = point,
                    branchlen = tip.branchlen,
                    status = %tip.status,
                    "FORK DETECTED: deep side branch"
                );
                if let Err(e) = db::record_fork_event(&self.db, point, &side, &tip.hash, &main, &view.active_hash) {
                    error!(error = %e, "Failed to record fork event");
                }
                let message = self.notifier.format_branch_alert(&view.name, tip, point, view.active_height);
                self.notify(&message).await;
            }
        }
    }

    /// Re-read each node's last REORG_WINDOW block hashes (a node, not an
    /// explorer API: about 30 quick calls) and alert when more than
    /// `max_branch_len` blocks it had were replaced since the last check.
    async fn check_reorgs(&self, endpoints: &[Endpoint]) {
        for ep in endpoints.iter().filter(|e| !e.rpc_url.contains("chainz.cryptoid.info")) {
            let client = endpoint_client(&ep.rpc_url);
            let now = match async {
                let tip = client.get_block_count().await?;
                let mut hashes = BTreeMap::new();
                for h in tip.saturating_sub(REORG_WINDOW - 1)..=tip {
                    hashes.insert(h, client.get_block_hash(h).await?);
                }
                anyhow::Ok(hashes)
            }
            .await
            {
                Ok(h) => h,
                Err(e) => {
                    warn!(endpoint = %ep.name, error = %e, "Reorg check: node unreachable");
                    continue;
                }
            };
            let before = match self.seen.lock() {
                Ok(mut seen) => seen.insert(ep.name.clone(), now.clone()),
                Err(_) => continue,
            };
            let reorg = before.as_ref().and_then(|b| find_reorg(b, &now));
            self.record_reorg_status(&ep.name, reorg);
            let Some(reorg) = reorg else { continue };
            if reorg.depth <= self.config.max_branch_len {
                info!(endpoint = %ep.name, fork_point = reorg.fork_point, depth = reorg.depth, "Reorg (normal)");
                continue;
            }
            warn!(endpoint = %ep.name, fork_point = reorg.fork_point, depth = reorg.depth, "FORK DETECTED: deep reorg");
            let new_hash = now.get(&(reorg.fork_point + 1)).cloned().unwrap_or_default();
            let old_hash = before
                .as_ref()
                .and_then(|b| b.get(&(reorg.fork_point + 1)).cloned())
                .unwrap_or_default();
            if let Err(e) = db::record_fork_event(
                &self.db,
                reorg.fork_point,
                &format!("{}: {}-block reorg, replaced", ep.name, reorg.depth),
                &old_hash,
                &format!("{}: new chain", ep.name),
                &new_hash,
            ) {
                error!(error = %e, "Failed to record fork event");
            }
            let tip = *now.keys().next_back().unwrap_or(&0);
            let message = self.notifier.format_reorg_alert(&ep.name, reorg.fork_point, reorg.depth, tip);
            self.notify(&message).await;
        }
    }

    fn record_reorg_status(&self, endpoint: &str, reorg: Option<Reorg>) {
        let Ok(mut status) = REORG_STATUS.lock() else { return };
        let now = Utc::now();
        let s = status.get_or_insert(ReorgStatus { checked_at: now, since: now, deepest: None });
        s.checked_at = now;
        if let Some(r) = reorg {
            if s.deepest.as_ref().is_none_or(|(_, d, _)| r.depth >= d.depth) {
                s.deepest = Some((endpoint.to_string(), r, now));
            }
        }
    }

    /// Send a fork alert to the fork watchers and the admins.
    async fn notify(&self, message: &str) {
        let mut notify_ids = db::get_fork_watchers(&self.db).unwrap_or_default();
        for &admin_id in &self.admin_ids {
            if !notify_ids.contains(&admin_id) {
                notify_ids.push(admin_id);
            }
        }
        if notify_ids.is_empty() {
            info!("Fork detected but no users subscribed for notifications");
            return;
        }
        info!(notify_count = notify_ids.len(), "Sending fork detection alerts");
        if let Err(e) = self.notifier.notify_users(&notify_ids, message).await {
            error!(error = %e, "Failed to send fork detection notifications");
        }
    }

    /// The endpoints' blocks at the same height: a node on its own chain.
    async fn compare_endpoints(&self, endpoints: &[Endpoint]) {

        if endpoints.len() < 2 {
            debug!(
                endpoint_count = endpoints.len(),
                "Need at least 2 endpoints for fork detection, skipping"
            );
            return;
        }

        let Some(cmp) = compare(endpoints).await else {
            debug!("Fewer than 2 reachable endpoints, skipping comparison");
            return;
        };
        let min_height = cmp.height;
        let hashes: HashMap<String, String> = cmp
            .views
            .into_iter()
            .filter_map(|v| Some((v.name, v.hash?)))
            .collect();

        if hashes.len() < 2 {
            debug!("Fewer than 2 hash responses, skipping comparison");
            return;
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
            return;
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

        warn!(height = min_height, "Sending fork alert for disagreeing endpoints");
        self.notify(&message).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(height: u64, branchlen: u64, status: &str) -> ChainTip {
        ChainTip { height, hash: format!("{height:064x}"), branchlen, status: status.to_string() }
    }

    #[test]
    fn deep_recent_branches_only() {
        let tips = vec![
            tip(4_254_316, 0, "active"),
            tip(4_254_108, 1, "valid-headers"),
            tip(4_247_983, 2, "valid-fork"),
            tip(4_254_300, 5, "valid-fork"),
            tip(4_200_000, 9, "valid-fork"), // more than a day old
        ];
        let v = BranchView::from_tips("services", &tips).unwrap();
        assert_eq!(v.active_height, 4_254_316);
        let deep = v.deep(3);
        assert_eq!(deep.len(), 1);
        assert_eq!(fork_point(deep[0]), 4_254_295);
        assert!(v.deep(5).is_empty());
    }

    fn chain(from: u64, hashes: &[&str]) -> BTreeMap<u64, String> {
        hashes.iter().enumerate().map(|(i, h)| (from + i as u64, h.to_string())).collect()
    }

    #[test]
    fn reorg_depth() {
        let before = chain(100, &["a", "b", "c", "d", "e"]);
        // two new blocks on top: no reorg
        assert_eq!(find_reorg(&before, &chain(102, &["c", "d", "e", "f", "g"])), None);
        // the last 3 blocks replaced, then one more
        assert_eq!(
            find_reorg(&before, &chain(101, &["b", "x", "y", "z", "w"])),
            Some(Reorg { fork_point: 101, depth: 3 })
        );
        // the new chain is shorter than the old one for now: still 4 replaced
        assert_eq!(
            find_reorg(&before, &chain(99, &["_", "a", "x", "y"])),
            Some(Reorg { fork_point: 100, depth: 4 })
        );
        // first check: nothing to compare
        assert_eq!(find_reorg(&BTreeMap::new(), &before), None);
    }

    #[test]
    fn no_active_tip_is_no_view() {
        assert!(BranchView::from_tips("x", &[tip(5, 1, "valid-fork")]).is_none());
    }
}
