//! The Addressables content catalog (`StreamingAssets/aa/catalog.json`):
//! which bundle holds an addressable asset. See `docs/formats/unity.md`.
//!
//! The JSON holds string lists (`m_InternalIds`, `m_ProviderIds`,
//! `m_resourceTypes`) and three base64 tables:
//! - keys: `i32` count, then per key a type byte and its value (type 0: `i32`
//!   length + ASCII; type 1: `i32` byte length + UTF-16LE; others ignored);
//! - buckets, one per key: `i32` offset of the key in the key table, `i32`
//!   entry count, that many `i32` entry indices;
//! - entries: `i32` count, then 7 `i32`s each: internal id, provider,
//!   dependency key (−1: none), dependency hash, extra data offset, primary
//!   key, resource type.

use std::collections::HashMap;

use crate::json::{Json, parse_json};
use crate::{Error, ErrorKind, Result};

fn invalid(offset: usize, what: impl Into<String>) -> Error {
    Error {
        offset,
        kind: ErrorKind::Invalid(what.into()),
    }
}

fn base64(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for (i, c) in text.bytes().enumerate() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return Err(invalid(i, "base64: bad character")),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

fn i32_at(data: &[u8], at: usize) -> Result<i32> {
    data.get(at..at + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| invalid(at, "catalog table ends early"))
}

/// One catalog entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    internal_id: usize,
    provider: usize,
    dependency_key: Option<usize>,
    resource_type: usize,
}

/// Where an asset is, with the locations it depends on (one level).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// For a bundle: its path (with Unity's `{…RuntimePath}` placeholder); for
    /// an asset inside a bundle: its build-time path (`Assets/…`).
    pub internal_id: String,
    /// Provider class name, e.g. `…AssetBundleProvider`.
    pub provider: String,
    /// Resource type class name, e.g. `UnityEngine.GameObject`.
    pub resource_type: String,
    pub dependencies: Vec<Location>,
}

/// The parsed catalog.
pub struct Catalog {
    internal_ids: Vec<String>,
    providers: Vec<String>,
    resource_types: Vec<String>,
    entries: Vec<Entry>,
    /// Per key (bucket): its entries.
    buckets: Vec<Vec<usize>>,
    /// String keys → bucket index.
    by_key: HashMap<String, usize>,
}

impl Catalog {
    pub fn parse(json: &[u8]) -> Result<Catalog> {
        let doc = parse_json(json)?;
        let strings = |name: &str| -> Result<Vec<String>> {
            match doc.get(name) {
                Some(Json::Array(items)) => Ok(items
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default().to_string())
                    .collect()),
                _ => Err(invalid(0, format!("catalog: no {name} list"))),
            }
        };
        let table = |name: &str| -> Result<Vec<u8>> {
            base64(
                doc.get(name)
                    .and_then(Json::as_str)
                    .ok_or_else(|| invalid(0, format!("catalog: no {name}")))?,
            )
        };
        let internal_ids = strings("m_InternalIds")?;
        let providers = strings("m_ProviderIds")?;
        let resource_types: Vec<String> = match doc.get("m_resourceTypes") {
            Some(Json::Array(items)) => items
                .iter()
                .map(|t| {
                    t.get("m_ClassName")
                        .and_then(Json::as_str)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect(),
            _ => return Err(invalid(0, "catalog: no m_resourceTypes")),
        };
        let keys = table("m_KeyDataString")?;
        let bucket_data = table("m_BucketDataString")?;
        let entry_data = table("m_EntryDataString")?;

        let count = |v: i32, at: usize, max: usize| {
            usize::try_from(v)
                .ok()
                .filter(|&n| n <= max)
                .ok_or_else(|| invalid(at, format!("catalog: bad count {v}")))
        };

        let n = count(i32_at(&entry_data, 0)?, 0, entry_data.len() / 28)?;
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let at = 4 + i * 28;
            let field = |k: usize| i32_at(&entry_data, at + 4 * k);
            let index = |v: i32, len: usize| usize::try_from(v).ok().filter(|&x| x < len);
            let entry = Entry {
                internal_id: index(field(0)?, internal_ids.len())
                    .ok_or_else(|| invalid(at, "catalog: bad internal id index"))?,
                provider: index(field(1)?, providers.len())
                    .ok_or_else(|| invalid(at, "catalog: bad provider index"))?,
                dependency_key: usize::try_from(field(2)?).ok(),
                resource_type: index(field(6)?, resource_types.len())
                    .ok_or_else(|| invalid(at, "catalog: bad resource type index"))?,
            };
            entries.push(entry);
        }

        let n = count(i32_at(&bucket_data, 0)?, 0, bucket_data.len() / 8)?;
        let mut buckets = Vec::with_capacity(n);
        let mut by_key = HashMap::new();
        let mut at = 4;
        for b in 0..n {
            let key_offset = usize::try_from(i32_at(&bucket_data, at)?)
                .map_err(|_| invalid(at, "catalog: bad key offset"))?;
            let m = count(i32_at(&bucket_data, at + 4)?, at + 4, bucket_data.len() / 4)?;
            at += 8;
            let mut list = Vec::with_capacity(m);
            for _ in 0..m {
                let e = usize::try_from(i32_at(&bucket_data, at)?)
                    .ok()
                    .filter(|&e| e < entries.len())
                    .ok_or_else(|| invalid(at, "catalog: bad entry index"))?;
                list.push(e);
                at += 4;
            }
            buckets.push(list);
            if let Some(key) = read_key(&keys, key_offset)? {
                by_key.entry(key).or_insert(b);
            }
        }
        for e in &entries {
            if e.dependency_key.is_some_and(|k| k >= buckets.len()) {
                return Err(invalid(0, "catalog: bad dependency key"));
            }
        }
        Ok(Catalog {
            internal_ids,
            providers,
            resource_types,
            entries,
            buckets,
            by_key,
        })
    }

    pub fn key_count(&self) -> usize {
        self.by_key.len()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn location(&self, entry: usize, depth: usize) -> Location {
        let e = self.entries[entry];
        let dependencies = match e.dependency_key {
            Some(k) if depth == 0 => self.buckets[k]
                .iter()
                .map(|&d| self.location(d, depth + 1))
                .collect(),
            _ => Vec::new(),
        };
        Location {
            internal_id: self.internal_ids[e.internal_id].clone(),
            provider: self.providers[e.provider].clone(),
            resource_type: self.resource_types[e.resource_type].clone(),
            dependencies,
        }
    }

    /// Every location registered under a string key (e.g. a prefab path
    /// like `WorldEntities/…/X.prefab`), with its dependencies.
    pub fn locate(&self, key: &str) -> Vec<Location> {
        self.by_key
            .get(key)
            .map(|&b| {
                self.buckets[b]
                    .iter()
                    .map(|&e| self.location(e, 0))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A string key at `offset` in the key table, or `None` for other key types.
fn read_key(keys: &[u8], offset: usize) -> Result<Option<String>> {
    let kind = *keys
        .get(offset)
        .ok_or_else(|| invalid(offset, "catalog: key offset out of range"))?;
    let len = || -> Result<usize> {
        usize::try_from(i32_at(keys, offset + 1)?)
            .map_err(|_| invalid(offset, "catalog: bad key length"))
    };
    let body = |n: usize| {
        keys.get(offset + 5..offset + 5 + n)
            .ok_or_else(|| invalid(offset, "catalog: key runs past the end"))
    };
    Ok(match kind {
        0 => Some(String::from_utf8_lossy(body(len()?)?).into_owned()),
        1 => {
            let bytes = body(len()?)?;
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let v = (u32::from(chunk[0]) << 16)
                | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
                | u32::from(*chunk.get(2).unwrap_or(&0));
            for k in 0..4 {
                if k <= chunk.len() {
                    out.push(T[(v >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A catalog with a prefab key whose entry depends on a bundle key.
    fn sample() -> String {
        let mut keys = 2i32.to_le_bytes().to_vec();
        let mut offsets = Vec::new();
        for k in ["bundle_a", "Things/Rock.prefab"] {
            offsets.push(keys.len() as i32);
            keys.push(0);
            keys.extend((k.len() as i32).to_le_bytes());
            keys.extend(k.as_bytes());
        }
        let mut buckets = 2i32.to_le_bytes().to_vec();
        for (offset, entry) in offsets.iter().zip([0i32, 1]) {
            for v in [*offset, 1, entry] {
                buckets.extend(v.to_le_bytes());
            }
        }
        let mut entries = 2i32.to_le_bytes().to_vec();
        for e in [[0i32, 0, -1, 0, 0, 0, 0], [1, 1, 0, 0, 0, 1, 1]] {
            for v in e {
                entries.extend(v.to_le_bytes());
            }
        }
        format!(
            r#"{{"m_InternalIds": ["{{Runtime}}\\a.bundle", "Assets/Things/Rock.prefab"],
                "m_ProviderIds": ["AssetBundleProvider", "BundledAssetProvider"],
                "m_resourceTypes": [{{"m_ClassName": "IAssetBundleResource"}}, {{"m_ClassName": "UnityEngine.GameObject"}}],
                "m_KeyDataString": "{}", "m_BucketDataString": "{}", "m_EntryDataString": "{}",
                "other": [1, -2.5e3, true, false, null, {{}}, [], "é"]}}"#,
            b64(&keys),
            b64(&buckets),
            b64(&entries)
        )
    }

    #[test]
    fn locates_an_asset_and_its_bundle() {
        let catalog = Catalog::parse(sample().as_bytes()).unwrap();
        assert_eq!(catalog.key_count(), 2);
        let found = catalog.locate("Things/Rock.prefab");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].internal_id, "Assets/Things/Rock.prefab");
        assert_eq!(found[0].resource_type, "UnityEngine.GameObject");
        assert_eq!(found[0].dependencies.len(), 1);
        assert_eq!(found[0].dependencies[0].internal_id, "{Runtime}\\a.bundle");
        assert!(catalog.locate("missing").is_empty());
    }

    #[test]
    fn base64_basics() {
        assert_eq!(base64("aGVsbG8=").unwrap(), b"hello");
        assert!(base64("a*").is_err());
    }

    #[test]
    fn corrupt_catalogs_never_panic() {
        let text = sample();
        let bytes = text.as_bytes();
        for i in 0..bytes.len() {
            let _ = Catalog::parse(&bytes[..i]);
            let mut bad = bytes.to_vec();
            bad[i] = b'0';
            let _ = Catalog::parse(&bad);
        }
    }
}
