// Stract is an open source web search engine.
// Copyright (C) 2023 Stract ApS
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

use anyhow::Result;
use file_store::temp::{TempDir, TempFile};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Seek, Write},
};

struct Chunk<T> {
    data: Vec<T>,
}

impl<T> Chunk<T> {
    fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn push(&mut self, item: T) {
        self.data.push(item);
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn sort(&mut self)
    where
        T: Ord,
    {
        self.data.sort();
    }

    fn write_into(self, file: &mut File) -> Result<()>
    where
        T: bincode::Encode,
    {
        let mut wrt = BufWriter::new(file);

        for item in self.data {
            let bytes = bincode::encode_to_vec(&item, common::bincode_config())?;
            let size = bytes.len() as u64;

            wrt.write_all(&size.to_le_bytes())?;
            wrt.write_all(&bytes)?;
        }

        wrt.flush()?;

        Ok(())
    }

    fn store(mut self, mut file: TempFile) -> Result<StoredChunk<T>>
    where
        T: bincode::Encode + Ord,
    {
        self.sort();
        self.write_into(file.inner_mut())?;

        StoredChunk::new(file)
    }
}

struct StoredChunk<T> {
    buf: Vec<u8>,
    data: BufReader<TempFile>,
    _marker: std::marker::PhantomData<T>,
}

impl<T> StoredChunk<T> {
    fn new(mut file: TempFile) -> Result<Self> {
        file.seek(std::io::SeekFrom::Start(0))?;
        Ok(Self {
            buf: Vec::new(),
            data: BufReader::new(file),
            _marker: std::marker::PhantomData,
        })
    }

    fn next(&mut self) -> Option<T>
    where
        T: bincode::Decode,
    {
        self.buf.clear();
        self.buf.resize(u64::BITS as usize / 8, 0);

        if self
            .data
            .fill_buf()
            .expect("external sort read failed")
            .is_empty()
        {
            return None;
        }
        self.data
            .read_exact(&mut self.buf)
            .expect("external sort truncated record");

        let next_size: u64 = u64::from_le_bytes(
            self.buf
                .as_slice()
                .try_into()
                .expect("external sort size field"),
        );

        self.buf.clear();
        assert!(
            next_size <= 64 * 1024 * 1024,
            "external sort record exceeds bound"
        );
        self.buf.resize(next_size as usize, 0);

        self.data
            .read_exact(&mut self.buf)
            .expect("external sort truncated payload");

        let (next, _) = bincode::decode_from_slice(&self.buf, common::bincode_config())
            .expect("external sort invalid payload");

        Some(next)
    }
}

pub struct ExternalSorter<T> {
    chunk_size: usize,
    _marker: std::marker::PhantomData<T>,
}

impl Default for ExternalSorter<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ExternalSorter<T> {
    pub fn new() -> Self {
        Self {
            chunk_size: 1_000_000,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    pub fn sort<I>(self, iter: I) -> Result<SortedIterator<T>>
    where
        I: Iterator<Item = T>,
        T: bincode::Encode + bincode::Decode + Ord,
    {
        let dir = TempDir::new()?;
        let mut levels: Vec<Vec<StoredChunk<T>>> = vec![Vec::new()];
        let mut chunk = Chunk {
            data: Vec::with_capacity(self.chunk_size.max(1)),
        };

        for item in iter {
            chunk.push(item);

            if chunk.len() >= self.chunk_size {
                let file = TempFile::new(&dir)?;
                let stored_chunk = chunk.store(file)?;
                push_chunk(&dir, &mut levels, stored_chunk)?;
                chunk = Chunk {
                    data: Vec::with_capacity(self.chunk_size.max(1)),
                };
            }
        }

        if !chunk.is_empty() {
            let file = TempFile::new(&dir)?;
            let stored_chunk = chunk.store(file)?;
            push_chunk(&dir, &mut levels, stored_chunk)?;
        }

        let mut stored_chunks: Vec<_> = levels.into_iter().flatten().collect();
        while stored_chunks.len() > 64 {
            let merge = stored_chunks.split_off(stored_chunks.len() - 64);
            stored_chunks.push(merge_chunks(&dir, merge)?);
        }

        let heads = stored_chunks
            .into_iter()
            .filter_map(|chunk| Head::new(chunk))
            .map(Reverse)
            .collect::<BinaryHeap<_>>();

        Ok(SortedIterator {
            _dir: dir,
            chunks: heads,
        })
    }
}

fn push_chunk<T: bincode::Encode + bincode::Decode + Ord>(
    dir: &TempDir,
    levels: &mut Vec<Vec<StoredChunk<T>>>,
    mut chunk: StoredChunk<T>,
) -> Result<()> {
    let mut level = 0;
    loop {
        if levels.len() <= level {
            levels.push(Vec::new());
        }
        levels[level].push(chunk);
        if levels[level].len() < 64 {
            return Ok(());
        }
        chunk = merge_chunks(dir, std::mem::take(&mut levels[level]))?;
        level += 1;
    }
}
fn merge_chunks<T: bincode::Encode + bincode::Decode + Ord>(
    dir: &TempDir,
    chunks: Vec<StoredChunk<T>>,
) -> Result<StoredChunk<T>> {
    let mut heads: BinaryHeap<_> = chunks
        .into_iter()
        .filter_map(Head::new)
        .map(Reverse)
        .collect();
    let mut file = TempFile::new(dir)?;
    {
        let mut writer = BufWriter::new(file.inner_mut());
        while let Some(Reverse(mut head)) = heads.pop() {
            let bytes = bincode::encode_to_vec(&head.item, common::bincode_config())?;
            writer.write_all(&(bytes.len() as u64).to_le_bytes())?;
            writer.write_all(&bytes)?;
            if let Some(item) = head.rest.next() {
                head.item = item;
                heads.push(Reverse(head));
            }
        }
        writer.flush()?;
    }
    StoredChunk::new(file)
}

struct Head<T> {
    item: T,
    rest: StoredChunk<T>,
}

impl<T> Head<T>
where
    T: bincode::Decode,
{
    fn new(mut chunk: StoredChunk<T>) -> Option<Self> {
        let item = chunk.next()?;

        Some(Self { item, rest: chunk })
    }
}

impl<T> Ord for Head<T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.item.cmp(&other.item)
    }
}

impl<T> PartialOrd for Head<T>
where
    T: Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Eq for Head<T> where T: Eq {}

impl<T> PartialEq for Head<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.item.eq(&other.item)
    }
}

impl<T> std::fmt::Debug for Head<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Head").field("item", &self.item).finish()
    }
}

pub struct SortedIterator<T> {
    // We need to keep the temp dir alive for the lifetime of the iterator
    _dir: TempDir,
    chunks: BinaryHeap<Reverse<Head<T>>>,
}

impl<T> Iterator for SortedIterator<T>
where
    T: bincode::Decode + Ord,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let mut head = self.chunks.pop()?.0;

        if let Some(next) = head.rest.next() {
            self.chunks.push(Reverse(Head {
                item: next,
                rest: head.rest,
            }));
        }

        Some(head.item)
    }
}

#[cfg(test)]
mod tests {
    use rand::Rng;

    use super::*;

    #[test]
    fn test_external_sort() {
        let mut rng = rand::thread_rng();

        let mut expected = Vec::new();
        let mut data = Vec::new();

        for _ in 0..1_000_000 {
            let item = rng.gen_range(0..1_000_000);

            expected.push(item);
            data.push(item);
        }

        expected.sort_unstable();

        let sorted = ExternalSorter::new()
            .with_chunk_size(100_000)
            .sort(data.into_iter())
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(sorted.len(), 1_000_000);
        assert_eq!(sorted, expected);
    }
}
