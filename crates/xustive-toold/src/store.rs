//! The cache both planes reach.
//!
//! Redis, because it is already in the topology and both networks can see it. Deliberately not a
//! shared filesystem or a database: this is a handful of small values with a natural expiry, and
//! the serving plane must be able to read them without any coupling to the fetcher's lifecycle.

use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};

use crate::{Cached, FetchError};

#[derive(Clone)]
pub struct Store {
    client: redis::Client,
}

impl Store {
    pub fn connect(url: &str) -> Result<Self, FetchError> {
        Ok(Self {
            client: redis::Client::open(url).map_err(|e| FetchError::Cache(e.to_string()))?,
        })
    }

    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection, FetchError> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| FetchError::Cache(e.to_string()))
    }

    /// Write a value, with an expiry well past its staleness limit.
    ///
    /// The expiry is a backstop, not the staleness rule. Letting Redis delete an entry at exactly
    /// the staleness limit would make "too old to show" and "never fetched" indistinguishable —
    /// and those need different messages, because one is a fetcher problem and the other is a
    /// cold start.
    pub async fn put<T: Serialize>(
        &self,
        key: &str,
        value: &Cached<T>,
        ttl_secs: u64,
    ) -> Result<(), FetchError> {
        let json = serde_json::to_string(value).map_err(|e| FetchError::Parse(e.to_string()))?;
        let mut conn = self.conn().await?;
        conn.set_ex::<_, _, ()>(key, json, ttl_secs)
            .await
            .map_err(|e| FetchError::Cache(e.to_string()))
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<Cached<T>>, FetchError> {
        let mut conn = self.conn().await?;
        let raw: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| FetchError::Cache(e.to_string()))?;
        let Some(raw) = raw else { return Ok(None) };
        match serde_json::from_str(&raw) {
            Ok(value) => Ok(Some(value)),
            Err(e) => {
                // A shape we cannot read is treated as absent rather than as an error. It means
                // an older build wrote it, and the correct response is to fetch again — not to
                // take the tool down until somebody clears a key by hand.
                tracing::warn!(key, error = %e, "unreadable cache entry; treating as absent");
                Ok(None)
            }
        }
    }

    pub async fn ping(&self) -> bool {
        match self.conn().await {
            Ok(mut conn) => redis::cmd("PING")
                .query_async::<String>(&mut conn)
                .await
                .is_ok(),
            Err(_) => false,
        }
    }
}
