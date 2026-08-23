//! KV access. Free tier allows 1000 writes/day against 280 rain ticks, so every
//! write here must be gated on an actual state transition.

use crate::domain::errors::ErrState;
use crate::domain::daily::DailyLog;
use crate::domain::quota::Quota;
use crate::domain::met::MetState;
use crate::domain::rules::Rule;
use crate::domain::state::EventState;
use serde::{de::DeserializeOwned, Serialize};
use worker::{Env, Error, Result};

pub const K_RULES: &str = "cfg:rules";
pub const K_EVENT: &str = "st:event";
pub const K_MET: &str = "st:met";
pub const K_ERR: &str = "st:err";
pub const K_QUOTA: &str = "st:quota";
pub const K_SNAPSHOT: &str = "st:snapshot";
pub const K_DAILY: &str = "st:daily";

pub struct Store {
    kv: worker::kv::KvStore,
}

fn kv_err(e: impl std::fmt::Display) -> Error {
    Error::RustError(format!("kv: {e}"))
}

impl Store {
    pub fn new(env: &Env) -> Result<Self> {
        Ok(Store { kv: env.kv("WX")? })
    }

    async fn get<T: DeserializeOwned + Default>(&self, key: &str) -> Result<T> {
        let raw = self.kv.get(key).text().await.map_err(kv_err)?;
        match raw {
            Some(s) => serde_json::from_str(&s)
                .map_err(|e| Error::RustError(format!("config parse {key}: {e}"))),
            None => Ok(T::default()),
        }
    }

    async fn put<T: Serialize>(&self, key: &str, v: &T) -> Result<()> {
        let s = serde_json::to_string(v).map_err(kv_err)?;
        self.kv.put(key, s).map_err(kv_err)?.execute().await.map_err(kv_err)
    }

    pub async fn rules(&self) -> Result<Vec<Rule>> {
        self.get::<Vec<Rule>>(K_RULES).await
    }
    pub async fn put_rules(&self, v: &[Rule]) -> Result<()> {
        self.put(K_RULES, &v).await
    }
    pub async fn event(&self) -> Result<EventState> {
        self.get(K_EVENT).await
    }
    pub async fn put_event(&self, v: &EventState) -> Result<()> {
        self.put(K_EVENT, v).await
    }
    pub async fn met(&self) -> Result<MetState> {
        self.get(K_MET).await
    }
    pub async fn put_met(&self, v: &MetState) -> Result<()> {
        self.put(K_MET, v).await
    }
    pub async fn errs(&self) -> Result<ErrState> {
        self.get(K_ERR).await
    }
    pub async fn put_errs(&self, v: &ErrState) -> Result<()> {
        self.put(K_ERR, v).await
    }
    pub async fn quota(&self) -> Result<Quota> {
        self.get(K_QUOTA).await
    }
    pub async fn put_quota(&self, v: &Quota) -> Result<()> {
        self.put(K_QUOTA, v).await
    }
    pub async fn daily(&self) -> Result<DailyLog> {
        self.get(K_DAILY).await
    }
    pub async fn put_daily(&self, v: &DailyLog) -> Result<()> {
        self.put(K_DAILY, v).await
    }
    pub async fn snapshot(&self) -> Result<Option<String>> {
        self.kv.get(K_SNAPSHOT).text().await.map_err(kv_err)
    }
    pub async fn put_snapshot(&self, s: &str) -> Result<()> {
        self.kv
            .put(K_SNAPSHOT, s.to_string())
            .map_err(kv_err)?
            .expiration_ttl(3600)
            .execute()
            .await
            .map_err(kv_err)
    }
}
