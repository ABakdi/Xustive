//! Near-duplicate detection by SimHash banding (M2-T05.3, M2-T05.6).
//!
//! Exact `content_hash` dedup ([[crate::dedup]]) catches only byte-identical bodies. Two accounts
//! of the same event, one reworded, hash to completely different values and slip through — yet they
//! are the same story, and a search result page showing both is worse for having found two.
//!
//! SimHash gives near bodies near hashes: a small edit flips a few bits, not all of them. So "is
//! this a near-duplicate" becomes "is there a stored hash within a few bits of this one". The
//! trouble is scale — you cannot compare a new hash against every stored one. Banding is the
//! standard escape.
//!
//! # The banding scheme, and why four bands
//!
//! Split the 64-bit hash into **four 16-bit bands** and index each document under all four band
//! values. To find near-duplicates of a new hash, look only in the buckets its own bands fall into.
//!
//! Four is not arbitrary. By the pigeonhole principle, two hashes that differ in **at most three
//! bits** cannot differ in all four bands — three bits can touch at most three of them — so at
//! least one band is identical and the pair lands in a shared bucket. Four bands therefore catch
//! **every** pair within Hamming distance 3 with no false negatives, which is the distance band
//! this project treats as "the same story" (M2-T05.6). Candidates from a shared bucket are then
//! confirmed by full Hamming distance, so a chance band collision costs one comparison, not a wrong
//! verdict.

use xustive_core::hash;

/// The number of bands, and the maximum Hamming distance they catch without false negatives.
///
/// Tied together on purpose: `BANDS` bands of `64 / BANDS` bits catch every pair within
/// `BANDS - 1` bits. Changing one without the other silently breaks the guarantee.
pub const BANDS: usize = 4;

/// The Hamming distance at or below which two bodies are treated as the same story.
///
/// Three, matching the band count minus one, so the banding index has no false negatives at this
/// distance. Above it — the 4-to-8 band — bodies are related but not the same; that is a cluster,
/// not a duplicate (M2-T05.6), and not collapsed here.
pub const NEAR_DISTANCE: u32 = 3;

/// The four 16-bit band values of a hash, high band first.
///
/// Pure and total. This is the whole indexing key: two hashes share a bucket iff they share a band
/// value here.
pub fn bands(h: u64) -> [u16; BANDS] {
    [
        (h >> 48) as u16,
        (h >> 32) as u16,
        (h >> 16) as u16,
        h as u16,
    ]
}

/// Whether two hashes are within [`NEAR_DISTANCE`] — the same story.
pub fn is_near(a: u64, b: u64) -> bool {
    hash::hamming(a, b) <= NEAR_DISTANCE
}

/// A SimHash banding index in Redis: band bucket → the hashes (and ids) that fell in it.
#[derive(Clone)]
pub struct SimHashIndex {
    client: redis::Client,
    namespace: String,
}

impl SimHashIndex {
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    fn bucket_key(&self, band: usize, value: u16) -> String {
        format!("{}:sim:{band}:{value}", self.namespace)
    }

    /// A stored candidate: its hash and the document it belongs to.
    fn encode(hash: u64, id: &str) -> String {
        format!("{hash:016x}\t{id}")
    }

    fn decode(raw: &str) -> Option<(u64, &str)> {
        let (h, id) = raw.split_once('\t')?;
        Some((u64::from_str_radix(h, 16).ok()?, id))
    }

    /// The id of a stored near-duplicate of `simhash`, if one exists.
    ///
    /// Looks only in the four buckets this hash falls into, confirms each candidate by full Hamming
    /// distance, and returns the first that is truly near. Fails **open** — an unreachable Redis
    /// yields `None` (no duplicate found), so a wobble lets a possible near-duplicate through rather
    /// than dropping a document. Same discipline as the exact dedup.
    pub async fn find_near(&self, simhash: u64) -> Option<String> {
        let mut conn = self.conn().await?;
        for (band, value) in bands(simhash).into_iter().enumerate() {
            let members: Vec<String> = redis::cmd("SMEMBERS")
                .arg(self.bucket_key(band, value))
                .query_async(&mut conn)
                .await
                .unwrap_or_default();
            for m in &members {
                if let Some((h, id)) = Self::decode(m) {
                    if is_near(simhash, h) {
                        return Some(id.to_string());
                    }
                }
            }
        }
        None
    }

    /// Index a document's hash under all four of its bands.
    ///
    /// Best-effort: a failed write means this document might not be found as a near-duplicate of a
    /// later one, which is a missed collapse, not a wrong verdict — acceptable, and it fails in the
    /// direction of keeping documents rather than dropping them.
    pub async fn insert(&self, simhash: u64, id: &str) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let member = Self::encode(simhash, id);
        let mut pipe = redis::pipe();
        for (band, value) in bands(simhash).into_iter().enumerate() {
            pipe.cmd("SADD")
                .arg(self.bucket_key(band, value))
                .arg(&member)
                .ignore();
        }
        let _: Result<(), _> = pipe.query_async::<()>(&mut conn).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_split_the_hash_into_four_16_bit_pieces() {
        let h = 0x1111_2222_3333_4444u64;
        assert_eq!(bands(h), [0x1111, 0x2222, 0x3333, 0x4444]);
    }

    /// The pigeonhole guarantee: any two hashes within three bits share at least one band. This is
    /// the property the whole index depends on for completeness, so it is checked directly across a
    /// spread of bit positions rather than assumed.
    #[test]
    fn hashes_within_three_bits_always_share_a_band() {
        let base = 0xDEAD_BEEF_CAFE_1234u64;
        // Flip every combination of up to three distinct bit positions.
        for i in 0..64 {
            for j in i..64 {
                for k in j..64 {
                    let flipped = base ^ (1 << i) ^ (1 << j) ^ (1 << k);
                    if hash::hamming(base, flipped) > NEAR_DISTANCE {
                        continue;
                    }
                    let (a, b) = (bands(base), bands(flipped));
                    let shares = a.iter().zip(b.iter()).any(|(x, y)| x == y);
                    assert!(
                        shares,
                        "hashes {base:016x} and {flipped:016x} are within {NEAR_DISTANCE} bits \
                         but share no band — the pigeonhole guarantee is broken"
                    );
                }
            }
        }
    }

    #[test]
    fn is_near_uses_the_distance_threshold() {
        let h = 0u64;
        assert!(is_near(h, 0b111)); // 3 bits
        assert!(!is_near(h, 0b1111)); // 4 bits
    }

    #[test]
    fn encode_decode_round_trips() {
        let enc = SimHashIndex::encode(0xABCD_1234_5678_9EF0, "doc-42");
        let (h, id) = SimHashIndex::decode(&enc).unwrap();
        assert_eq!(h, 0xABCD_1234_5678_9EF0);
        assert_eq!(id, "doc-42");
    }
}
