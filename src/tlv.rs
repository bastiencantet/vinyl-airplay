//! TLV8 for the single-record HAP pairing exchanges (not pairing lists).
use anyhow::{Result, bail, ensure};
use std::collections::BTreeMap;

pub fn encode(fields: &[(u8, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(tag, value) in fields {
        if value.is_empty() {
            out.extend_from_slice(&[tag, 0]);
        }
        for chunk in value.chunks(255) {
            out.extend_from_slice(&[tag, chunk.len() as u8]);
            out.extend_from_slice(chunk);
        }
    }
    out
}

pub fn decode(data: &[u8]) -> Result<BTreeMap<u8, Vec<u8>>> {
    let mut fields: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
    let mut pos = 0;
    let mut previous = None;
    while pos < data.len() {
        ensure!(pos + 2 <= data.len(), "truncated TLV header");
        let tag = data[pos];
        let len = data[pos + 1] as usize;
        pos += 2;
        ensure!(pos + len <= data.len(), "truncated TLV value");
        if fields.contains_key(&tag) {
            ensure!(
                previous == Some((tag, 255)),
                "duplicate TLV field outside continuation"
            );
        }
        fields
            .entry(tag)
            .or_default()
            .extend_from_slice(&data[pos..pos + len]);
        previous = Some((tag, len));
        pos += len;
    }
    if let Some(error) = fields.get(&7) {
        bail!(
            "pairing rejected: HAP error {error:?}, backoff {:?}; no automatic retry",
            fields.get(&8)
        );
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_public_key_and_empty_value() {
        let key = vec![42; 384];
        let bytes = encode(&[(3, &key), (6, &[2]), (9, &[])]);
        assert_eq!(&bytes[..2], &[3, 255]);
        assert_eq!(&bytes[257..259], &[3, 129]);
        let fields = decode(&bytes).unwrap();
        assert_eq!(fields[&3], key);
        assert!(fields[&9].is_empty());
    }
    #[test]
    fn rejects_truncation_duplicates_and_pairing_errors() {
        for invalid in [&[3][..], &[3, 2, 0], &[6, 1, 2, 6, 1, 3], &[7, 1, 2]] {
            assert!(decode(invalid).is_err());
        }
    }
}
