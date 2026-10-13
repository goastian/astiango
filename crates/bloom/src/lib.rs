// Stract is an open source web search engine.
// Copyright (C) 2024 Stract ApS
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use bitvec::vec::BitVec;

pub fn combine_u64s(nums: [u64; 2]) -> u128 {
    ((nums[0] as u128) << 64) | (nums[1] as u128)
}

pub fn split_u128(num: u128) -> [u64; 2] {
    [(num >> 64) as u64, num as u64]
}

const XXH3_SECRET: &[u8] = &xxhash_rust::const_xxh3::const_custom_default_secret(42);
pub fn fast_stable_hash_64(t: &[u8]) -> u64 {
    xxhash_rust::xxh3::xxh3_64_with_secret(t, XXH3_SECRET)
}

pub fn fast_stable_hash_128(t: &[u8]) -> u128 {
    xxhash_rust::xxh3::xxh3_128_with_secret(t, XXH3_SECRET)
}

const LARGE_PRIME: u64 = 11400714819323198549;

/// Calculate the number of bits needed for a Bloom filter.
#[inline]
fn num_bits(estimated_items: u64, fp: f64) -> u64 {
    assert!(fp.is_finite() && fp > 0.0 && fp < 1.0);
    ((estimated_items.max(1) as f64) * -fp.ln() / 2.0_f64.ln().powi(2)).ceil() as u64
}

/// Calculate the number of hash functions needed for a Bloom filter.
#[inline]
fn num_hashes(num_bits: u64, estimated_items: u64) -> u64 {
    (((num_bits as f64) / estimated_items as f64 * 2.0_f64.ln()).round() as u64).max(1)
}

#[derive(
    Clone,
    bincode::Encode,
    bincode::Decode,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Eq,
)]
pub struct U64BloomFilter {
    #[bincode(with_serde)]
    bit_vec: BitVec,
}

impl U64BloomFilter {
    pub fn new(estimated_items: u64, fp: f64) -> Self {
        Self::with_memory_limit(estimated_items, fp, 128 * 1024 * 1024)
    }
    pub fn with_memory_limit(estimated_items: u64, fp: f64, max_bytes: usize) -> Self {
        assert!(fp.is_finite() && fp > 0.0 && fp < 1.0 && max_bytes >= 8);
        let num_bits = ((-(estimated_items.max(1) as f64) / (-fp).ln_1p()).ceil() as usize)
            .min(max_bytes.saturating_mul(8))
            .max(1);
        Self {
            bit_vec: BitVec::repeat(false, num_bits),
        }
    }

    pub fn empty_from(other: &Self) -> Self {
        Self {
            bit_vec: BitVec::repeat(false, other.bit_vec.len()),
        }
    }

    pub fn fill(&mut self) {
        for i in 0..self.bit_vec.len() {
            self.bit_vec.set(i, true);
        }
    }

    fn hash(item: u64) -> usize {
        item.wrapping_mul(LARGE_PRIME) as usize
    }

    pub fn insert(&mut self, item: u64) {
        let h = Self::hash(item);
        let num_bits = self.bit_vec.len();
        self.bit_vec.set(h % num_bits, true);
    }

    pub fn insert_u128(&mut self, item: u128) {
        self.insert(item as u64) // TODO: properly support u128
    }

    pub fn contains(&self, item: u64) -> bool {
        let h = Self::hash(item);
        self.bit_vec[h % self.bit_vec.len()]
    }

    pub fn contains_u128(&self, item: u128) -> bool {
        self.contains(item as u64) // TODO: properly support u128
    }

    pub fn estimate_card(&self) -> u64 {
        let num_ones = self.bit_vec.count_ones() as u64;

        if num_ones == 0 || self.bit_vec.is_empty() {
            return 0;
        }

        if num_ones == self.bit_vec.len() as u64 {
            return u64::MAX;
        }

        (-(self.bit_vec.len() as f64) * (1.0 - num_ones as f64 / self.bit_vec.len() as f64).ln())
            .round() as u64
    }

    pub fn union(&mut self, other: Self) {
        debug_assert_eq!(self.bit_vec.len(), other.bit_vec.len());

        self.bit_vec |= other.bit_vec;
    }
}

#[derive(bincode::Encode, bincode::Decode)]
pub struct BytesBloomFilter<T> {
    #[bincode(with_serde)]
    bit_vec: BitVec,
    num_hashes: u64,
    _marker: std::marker::PhantomData<T>,
}

impl<T> BytesBloomFilter<T> {
    pub fn new(estimated_items: u64, fp: f64) -> Self {
        let num_bits = num_bits(estimated_items, fp);
        let num_hashes = num_hashes(num_bits, estimated_items.max(1));
        Self {
            bit_vec: BitVec::repeat(false, num_bits as usize),
            num_hashes,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_memory_limit(estimated_items: u64, fp: f64, max_bytes: usize) -> Self {
        assert!(max_bytes >= std::mem::size_of::<usize>());
        let bits = num_bits(estimated_items.max(1), fp)
            .min((max_bytes as u64) * 8)
            .max(8);
        Self {
            bit_vec: BitVec::repeat(false, bits as usize),
            num_hashes: num_hashes(bits, estimated_items.max(1)),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn bits(&self) -> usize {
        self.bit_vec.len()
    }
    pub fn allocated_bytes(&self) -> usize {
        self.bit_vec.as_raw_slice().len() * std::mem::size_of::<usize>()
    }
    pub fn hashes(&self) -> u64 {
        self.num_hashes
    }
    pub fn fill(&mut self) {
        self.bit_vec.fill(true);
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()>
    where
        T: bincode::Encode,
    {
        use std::io::Write;
        let bytes = bincode::encode_to_vec(self, bincode::config::standard())
            .map_err(std::io::Error::other)?;
        let path = path.as_ref();
        let temp = path.with_extension("bloom.tmp");
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(b"ASTIAN-BLOOM\x02")?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temp, path)?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }

    pub fn load(path: impl AsRef<std::path::Path>, max_bytes: usize) -> std::io::Result<Self>
    where
        T: bincode::Decode,
    {
        let path = path.as_ref();
        if path.metadata()?.len() > (max_bytes as u64).saturating_add(4096) {
            return Err(std::io::Error::other("Bloom exceeds configured memory"));
        }
        let data = std::fs::read(path)?;
        let prefix = b"ASTIAN-BLOOM\x02";
        if !data.starts_with(prefix) {
            return Err(std::io::Error::other(
                "unsupported Bloom version; rebuild from exact keys",
            ));
        }
        let (filter, consumed): (Self, usize) =
            bincode::decode_from_slice(&data[prefix.len()..], bincode::config::standard())
                .map_err(std::io::Error::other)?;
        if consumed + prefix.len() != data.len()
            || filter.bits() == 0
            || filter.hashes() == 0
            || filter.hashes() > 64
            || filter.allocated_bytes() > max_bytes
        {
            return Err(std::io::Error::other("invalid Bloom dimensions"));
        }
        Ok(filter)
    }

    fn hash_raw(item: &[u8]) -> [u64; 2] {
        split_u128(fast_stable_hash_128(item))
    }

    pub fn contains_raw(&self, item: &[u8]) -> bool {
        let [a, b] = Self::hash_raw(item);

        for i in 0..self.num_hashes {
            let h = ((a.wrapping_mul(i).wrapping_add(b)) % LARGE_PRIME) % self.bit_vec.len() as u64;
            if !self.bit_vec[h as usize] {
                return false;
            }
        }

        true
    }

    pub fn insert_raw(&mut self, item: &[u8]) {
        // see https://en.wikipedia.org/wiki/Universal_hashing#Hashing_integers
        // for why this universal hash construction works
        let [a, b] = Self::hash_raw(item);

        for i in 0..self.num_hashes {
            let h = ((a.wrapping_mul(i).wrapping_add(b)) % LARGE_PRIME) % self.bit_vec.len() as u64;
            self.bit_vec.set(h as usize, true);
        }
    }
}

impl<T> BytesBloomFilter<T>
where
    T: AsRef<[u8]>,
{
    pub fn insert(&mut self, item: &T) {
        self.insert_raw(item.as_ref())
    }

    pub fn contains(&self, item: &T) -> bool {
        self.contains_raw(item.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_filter() {
        let mut bf = U64BloomFilter::new(100, 0.01);
        bf.insert(1);
        bf.insert(2);
        bf.insert(3);
        bf.insert(4);
        bf.insert(5);

        assert!(bf.contains(1));
        assert!(bf.contains(2));
        assert!(bf.contains(3));
        assert!(bf.contains(4));
        assert!(bf.contains(5));
        assert!(!bf.contains(6));
        assert!(!bf.contains(7));
        assert!(!bf.contains(8));
        assert!(!bf.contains(9));
        assert!(!bf.contains(10));
    }

    #[test]
    fn test_bloom_filter_bytes() {
        let mut bf = BytesBloomFilter::new(100, 0.01);
        bf.insert(&1u64.to_le_bytes());
        bf.insert(&2u64.to_le_bytes());
        bf.insert(&3u64.to_le_bytes());
        bf.insert(&4u64.to_le_bytes());
        bf.insert(&5u64.to_le_bytes());

        assert!(bf.contains(&1u64.to_le_bytes()));
        assert!(bf.contains(&2u64.to_le_bytes()));
        assert!(bf.contains(&3u64.to_le_bytes()));
        assert!(bf.contains(&4u64.to_le_bytes()));
        assert!(bf.contains(&5u64.to_le_bytes()));
        assert!(!bf.contains(&6u64.to_le_bytes()));
        assert!(!bf.contains(&7u64.to_le_bytes()));
        assert!(!bf.contains(&8u64.to_le_bytes()));
        assert!(!bf.contains(&9u64.to_le_bytes()));
        assert!(!bf.contains(&10u64.to_le_bytes()));
    }

    #[test]
    fn split_combine_u128() {
        for num in 0..10000_u128 {
            assert_eq!(combine_u64s(split_u128(num)), num);
        }
    }
}

#[cfg(test)]
mod sizing_tests {
    use super::*;
    #[test]
    fn independent_false_positives_and_allocated_memory() {
        for n in [1000u64, 10_000, 100_000] {
            let mut filter = BytesBloomFilter::<String>::new(n, 0.01);
            for i in 0..n {
                filter.insert_raw(&i.to_le_bytes());
            }
            assert_eq!(filter.bits(), num_bits(n, 0.01) as usize);
            assert_eq!(
                filter.allocated_bytes(),
                filter.bits().div_ceil(usize::BITS as usize) * std::mem::size_of::<usize>()
            );
            let samples = 100_000u64;
            let false_positives = (n..n + samples)
                .filter(|i| filter.contains_raw(&i.to_le_bytes()))
                .count();
            assert!(
                false_positives <= 1400,
                "capacity {n}: {false_positives}/100000"
            );
            assert!((0..n).all(|i| filter.contains_raw(&i.to_le_bytes())));
        }
    }
    #[test]
    fn memory_cap_and_versioned_roundtrip() {
        let mut filter = BytesBloomFilter::<String>::with_memory_limit(10_000_000_000, 0.01, 8192);
        assert_eq!(filter.allocated_bytes(), 8192);
        filter.insert_raw(b"known");
        let path = std::env::temp_dir().join(format!("bloom-{}.v2", std::process::id()));
        filter.save(&path).unwrap();
        let loaded = BytesBloomFilter::<String>::load(&path, 8192).unwrap();
        assert!(loaded.contains_raw(b"known"));
        assert!(BytesBloomFilter::<String>::load(&path, 1024).is_err());
        std::fs::write(&path, b"ASTIAN-BLOOM\x01").unwrap();
        assert!(BytesBloomFilter::<String>::load(&path, 8192).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
