//! What both transports share: the open databases, who may reach which one, and the metrics.
//!
//! With `--db` there is one database and no token. With `--data-dir` every tenant named in the
//! tokens file has its own database in `<data-dir>/<tenant>`, opened at start, and a request
//! reaches the database of its token's tenant and no other: no route or field names a tenant, so
//! a caller cannot even ask for another one.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use kerno_backup::{ObjectStore, Prefixed};

use crate::args::{Backup, Store};
use crate::auth::{Tokens, bearer};
use crate::error::ApiError;
use crate::metrics::{BackupTotals, Metrics};
use crate::service::{Res, Service};

pub struct App {
    /// Every open database by tenant; the one database of `--db` is the tenant `""`.
    dbs: BTreeMap<String, Arc<Service>>,
    /// `None` with `--db`: every request reaches its one database.
    tokens: Option<Tokens>,
    pub metrics: Metrics,
    /// How often to ship; `None` when backups are off.
    backup_every: Option<Duration>,
}

impl App {
    /// Opens every database `store` names, each with its backups under `backup`'s location when
    /// there is one (a tenant's under its own name).
    pub fn open(store: &Store, backup: Option<&Backup>, fulltext: &[String]) -> Result<Self, String> {
        let location: Option<Arc<dyn ObjectStore>> = backup
            .map(|b| {
                kerno_backup::open_store(&b.url).map_err(|e| format!("cannot use the backup location {}: {e}", b.url))
            })
            .transpose()?;
        let open = |dir: &std::path::Path, tenant: &str| -> Result<Arc<Service>, String> {
            let svc = Service::open(dir, fulltext)?;
            Ok(Arc::new(match &location {
                Some(loc) => svc.with_backups(Arc::new(Prefixed::new(Arc::clone(loc), tenant))),
                None => svc,
            }))
        };
        let (dbs, tokens) = match store {
            Store::Db(dir) => (BTreeMap::from([(String::new(), open(dir, "")?)]), None),
            Store::Tenants { root, tokens } => {
                let tokens = Tokens::load(tokens)?;
                let mut dbs = BTreeMap::new();
                for tenant in tokens.tenants() {
                    // `Tokens` admits only [a-z0-9_-]{1,64}, so this stays inside `root`. Opening
                    // creates the missing directories, `root` included.
                    dbs.insert(tenant.to_owned(), open(&root.join(tenant), tenant)?);
                    tracing::info!(tenant, "tenant database open");
                }
                (dbs, Some(tokens))
            }
        };
        Ok(App { dbs, tokens, metrics: Metrics::default(), backup_every: backup.map(|b| b.interval) })
    }

    /// How often the backups are shipped; `None` when they are off.
    pub fn backup_interval(&self) -> Option<Duration> {
        self.backup_every
    }

    /// Ships every database's new commits, taking base backups where they are needed. A failure
    /// is logged and counted and does not stop the others: the next tick tries again.
    pub async fn backup_tick(&self) {
        for (tenant, svc) in &self.dbs {
            if svc.backup_stats().is_none() {
                continue;
            }
            match svc.run(Service::backup_tick).await {
                Ok(tick) => {
                    let shipped = tick.shipped;
                    if tick.base.is_some() || shipped.commits > 0 {
                        tracing::info!(
                            tenant,
                            base_commit = tick.base.as_ref().map(|b| b.seq),
                            shipped_commits = shipped.commits,
                            shipped_to = shipped.shipped_seq,
                            "backup"
                        );
                    }
                }
                // `run` logged the cause.
                Err(_) => tracing::warn!(tenant, "backup failed; trying again at the next tick"),
            }
        }
    }

    /// What the backups of every database did, `None` when they are off.
    pub fn backup_totals(&self) -> Option<BackupTotals> {
        self.backup_every?;
        let mut t = BackupTotals::default();
        for stats in self.dbs.values().filter_map(|s| s.backup_stats()) {
            t.commits += stats.commits.load(Ordering::Relaxed);
            t.segments += stats.segments.load(Ordering::Relaxed);
            t.bases += stats.bases.load(Ordering::Relaxed);
            t.errors += stats.errors.load(Ordering::Relaxed);
        }
        Some(t)
    }

    /// Whether a request needs a token.
    pub fn authenticates(&self) -> bool {
        self.tokens.is_some()
    }

    /// The tenant and database of a request whose `Authorization` header is `authorization`.
    pub fn service(&self, authorization: Option<&[u8]>) -> Res<(&str, Arc<Service>)> {
        let tenant = match &self.tokens {
            None => "",
            Some(tokens) => {
                let token = bearer(authorization);
                match token.and_then(|t| tokens.tenant_of(t)) {
                    Some(tenant) => tenant,
                    None => {
                        self.metrics.auth_failure();
                        // Never the token itself, not even a prefix.
                        let reason = if token.is_some() { "unknown token" } else { "no bearer token" };
                        tracing::warn!(reason, "request refused");
                        return Err(ApiError::unauthenticated());
                    }
                }
            }
        };
        let svc = self.dbs.get(tenant).ok_or_else(|| ApiError::internal(format!("tenant {tenant:?} is not open")))?;
        Ok((tenant, Arc::clone(svc)))
    }

    /// Ready when every database is.
    pub async fn ready(&self) -> Res<()> {
        for svc in self.dbs.values() {
            svc.run(Service::ready).await?;
        }
        Ok(())
    }

    pub fn databases(&self) -> usize {
        self.dbs.len()
    }

    /// Closes every database, reporting the first failure. A database still shared by a handler
    /// thread that is finishing is closed when that thread drops it, only without a report.
    pub fn close(self) -> Result<(), String> {
        let mut first = Ok(());
        for svc in self.dbs.into_values() {
            if let Ok(svc) = Arc::try_unwrap(svc) {
                let closed = svc.close();
                if first.is_ok() {
                    first = closed;
                }
            }
        }
        first
    }
}
