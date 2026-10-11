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

use anyhow::Result;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub struct Wal<T> {
    writer: file_store::iterable::IterableStoreWriter<T, File>,
    path: PathBuf,
    sync_file: File,
}

impl<T> Wal<T> {
    pub fn open<P: AsRef<Path>>(file: P) -> Result<Self> {
        let path = file.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;
        let sync_file = file.try_clone()?;
        Ok(Wal {
            writer: file_store::iterable::IterableStoreWriter::new(file),
            path,
            sync_file,
        })
    }

    pub fn clear(&mut self) -> Result<()> {
        std::fs::remove_file(&self.path)?;
        *self = Self::open(&self.path)?;

        Ok(())
    }
}

impl<T> Wal<T>
where
    T: bincode::Encode,
{
    pub fn write(&mut self, item: &T) -> Result<()> {
        self.writer.write(item)?;
        self.writer.flush()?;

        Ok(())
    }

    /// Confirm that flushed records and the directory entry have reached storage.
    pub fn checkpoint(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.sync_file.sync_all()?;
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    pub fn batch_write_durable<'a>(&mut self, items: impl Iterator<Item = &'a T>) -> Result<()>
    where
        T: 'a,
    {
        self.batch_write(items)?;
        self.checkpoint()
    }

    pub fn batch_write<'a>(&mut self, items: impl Iterator<Item = &'a T>) -> Result<()>
    where
        T: 'a,
    {
        for item in items {
            self.writer.write(item)?;
        }

        self.writer.flush()?;

        Ok(())
    }
}

impl<T> Wal<T>
where
    T: bincode::Decode,
{
    pub fn iter(&self) -> Result<WalIterator<T>> {
        WalIterator::open(&self.path)
    }
}

pub struct WalIterator<T> {
    iter: file_store::iterable::IterableStoreReader<T>,
}

impl<T> WalIterator<T> {
    pub fn open<P: AsRef<Path>>(file: P) -> Result<Self> {
        let iter = file_store::iterable::IterableStoreReader::open(file)?;
        Ok(Self { iter })
    }
}

impl<T> Iterator for WalIterator<T>
where
    T: bincode::Decode,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_write_read() -> Result<()> {
        let temp_dir = file_store::gen_temp_dir().unwrap();
        let mut writer = Wal::open(temp_dir.as_ref().join("test-wal"))?;

        writer.write(&1u64)?;
        writer.write(&2u64)?;
        writer.write(&3u64)?;

        let res: Vec<_> = writer.iter()?.collect();

        assert_eq!(&res, &[1, 2, 3]);

        writer.write(&4u64)?;

        let res: Vec<_> = writer.iter()?.collect();

        assert_eq!(&res, &[1, 2, 3, 4]);

        Ok(())
    }

    #[test]
    fn test_empty_write() -> Result<()> {
        let temp_dir = file_store::gen_temp_dir().unwrap();
        let writer: Wal<u64> = Wal::open(temp_dir.as_ref().join("test-wal"))?;

        let res: Vec<_> = writer.iter()?.collect();

        assert!(res.is_empty());

        Ok(())
    }

    #[test]
    fn test_clear() -> Result<()> {
        let temp_dir = file_store::gen_temp_dir().unwrap();
        let mut writer = Wal::open(temp_dir.as_ref().join("test-wal"))?;

        writer.write(&1u64)?;
        writer.write(&2u64)?;
        writer.write(&3u64)?;

        let res: Vec<_> = writer.iter()?.collect();

        assert_eq!(&res, &[1, 2, 3]);

        writer.clear()?;

        let res: Vec<_> = writer.iter()?.collect();
        assert!(res.is_empty());

        Ok(())
    }

    #[test]
    fn durable_batches_survive_reopen_and_append() -> Result<()> {
        let directory = file_store::gen_temp_dir()?;
        let path = directory.as_ref().join("wal");
        let mut wal = Wal::open(&path)?;
        wal.batch_write_durable([1u64, 2].iter())?;
        drop(wal);
        let mut wal = Wal::open(&path)?;
        wal.batch_write_durable([3u64, 4].iter())?;
        drop(wal);
        assert_eq!(
            Wal::<u64>::open(&path)?.iter()?.collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        Ok(())
    }

    #[test]
    fn clear_replaces_the_file_used_by_the_next_durable_checkpoint() -> Result<()> {
        let directory = file_store::gen_temp_dir()?;
        let path = directory.as_ref().join("wal");
        let mut wal = Wal::open(&path)?;
        wal.batch_write_durable([1u64, 2].iter())?;
        wal.clear()?;
        wal.batch_write_durable([3u64].iter())?;
        drop(wal);
        assert_eq!(Wal::<u64>::open(&path)?.iter()?.collect::<Vec<_>>(), [3]);
        Ok(())
    }

    #[test]
    fn durable_write_propagates_a_flush_failure() -> Result<()> {
        let directory = file_store::gen_temp_dir()?;
        let path = directory.as_ref().join("wal");
        File::create(&path)?;
        let file = File::open(&path)?;
        let mut wal = Wal {
            sync_file: file.try_clone()?,
            writer: file_store::iterable::IterableStoreWriter::new(file),
            path,
        };
        assert!(wal.batch_write_durable([1u64].iter()).is_err());
        Ok(())
    }

    #[test]
    fn durable_checkpoint_propagates_a_missing_parent_directory() -> Result<()> {
        let directory = file_store::gen_temp_dir()?;
        let parent = directory.as_ref().join("removed");
        std::fs::create_dir(&parent)?;
        let path = parent.join("wal");
        let mut wal = Wal::open(&path)?;
        wal.write(&1u64)?;
        std::fs::remove_file(&path)?;
        std::fs::remove_dir(parent)?;
        assert!(wal.checkpoint().is_err());
        Ok(())
    }
}
