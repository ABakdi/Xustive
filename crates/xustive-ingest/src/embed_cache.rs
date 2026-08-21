//! A `phash → CLIP vector` reuse cache in Redis ([[Vector Index]] §5, M3-T05.3).
//!
//! The same image is reposted all over the web at different URLs — a logo, a viral screenshot, a
//! stock photo. Each copy fetches to different bytes at a different URL, but its *structure* is
//! identical, so its perceptual hash ([`xustive_media::phash`]) is identical. This cache keys the
//! CLIP embedding on that hash: the first copy pays for the embedding, every later copy reads the
//! vector back instead of calling the model again.
//!
//! # Why this is worth a cache
//!
//! The embedding is the expensive step — a model call per image. The fetch and the hash are cheap
//! by comparison. Reusing the vector for a known hash turns "embed every image ever crawled" into
//! "embed every *distinct* image", which on a real corpus of reposted media is a large saving.
//!
//! # Correctness
//!
//! The reused vector is still upserted as a **new point** with this image's own URL and document in
//! its payload — reuse saves the model call, it does not collapse two documents into one. Entries
//! carry a TTL so a hash that stops recurring eventually ages out rather than pinning memory forever.

use std::time::Duration;

/// A Redis-backed `phash → vector` store.
#[derive(Clone)]
pub struct EmbedCache {
    manager: redis::aio::ConnectionManager,
    namespace: String,
    ttl: Duration,
}

impl EmbedCache {
    /// Connect within a namespace. `None` if Redis is unreachable — the cache is an optimisation,
    /// and its absence only means images are always embedded, never that anything breaks.
    pub async fn connect_in(url: &str, namespace: &str, ttl: Duration) -> Option<Self> {
        let client = redis::Client::open(url).ok()?;
        let manager = client.get_connection_manager().await.ok()?;
        Some(Self {
            manager,
            namespace: namespace.to_string(),
            ttl,
        })
    }

    fn key(&self, phash: &str) -> String {
        format!("{}:vecphash:{phash}", self.namespace)
    }

    fn ttl_secs(&self) -> u64 {
        self.ttl.as_secs().max(1)
    }

    /// The cached vector for a hash, or `None` if not present (or malformed, or Redis is down).
    pub async fn get(&self, phash: &str) -> Option<Vec<f32>> {
        let mut conn = self.manager.clone();
        let bytes: Option<Vec<u8>> = redis::cmd("GET")
            .arg(self.key(phash))
            .query_async(&mut conn)
            .await
            .ok()?;
        decode_vec(&bytes?)
    }

    /// Cache a vector for a hash, with the configured TTL. Failure is swallowed — a cache write that
    /// does not happen costs a future model call, nothing more.
    pub async fn put(&self, phash: &str, vector: &[f32]) {
        let mut conn = self.manager.clone();
        let _: Result<(), _> = redis::cmd("SET")
            .arg(self.key(phash))
            .arg(encode_vec(vector))
            .arg("EX")
            .arg(self.ttl_secs())
            .query_async::<()>(&mut conn)
            .await;
    }
}

/// Pack a vector as little-endian f32 bytes — 4 bytes per dimension, no framing. Compact (a 512-d
/// vector is 2 KB) and exact: floats round-trip bit-for-bit, unlike a decimal text encoding.
pub fn encode_vec(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Unpack little-endian f32 bytes. `None` if the length is not a whole number of floats — a
/// truncated or foreign value is treated as a miss, not decoded into garbage.
pub fn decode_vec(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trips_exactly() {
        let v = vec![0.0, 1.0, -1.5, 3.14159, f32::MIN_POSITIVE, -0.0];
        let back = decode_vec(&encode_vec(&v)).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn a_512d_vector_encodes_to_2kb() {
        let v = vec![0.25f32; 512];
        assert_eq!(encode_vec(&v).len(), 512 * 4);
    }

    #[test]
    fn a_truncated_value_is_a_miss_not_garbage() {
        assert!(decode_vec(&[1, 2, 3]).is_none()); // not a multiple of 4
        assert!(decode_vec(&[]).is_none());
    }

    #[test]
    fn decode_reads_the_bytes_encode_wrote() {
        // A single known float, checked at the byte level so the endianness is pinned.
        let bytes = encode_vec(&[1.0]);
        assert_eq!(bytes, vec![0x00, 0x00, 0x80, 0x3f]); // 1.0f32 little-endian
        assert_eq!(decode_vec(&bytes), Some(vec![1.0]));
    }
}
