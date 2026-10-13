use crate::Result;
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

pub struct Workspace {
    pub jobs: PathBuf,
    root: PathBuf,
    output: PathBuf,
    _lock: File,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    version: u32,
    staged: PathBuf,
    backup: PathBuf,
    complete: bool,
}

impl Workspace {
    pub fn open(output: &Path) -> Result<Self> {
        let root = PathBuf::from(format!("{}.consumer-work", output.display()));
        let jobs = root.join("jobs");
        fs::create_dir_all(&jobs)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("workspace.lock"))?;
        lock.try_lock_exclusive()?;
        let workspace = Self {
            jobs,
            root,
            output: output.into(),
            _lock: lock,
        };
        workspace.recover()?;
        Ok(workspace)
    }

    pub fn key(source: &crate::config::WarcSource, path: &str) -> Result<String> {
        let identity = match source {
            crate::config::WarcSource::Local(c) => format!("local:{}:{path}", c.folder),
            crate::config::WarcSource::HTTP(c) => format!("http:{}:{path}", c.base_url),
            crate::config::WarcSource::S3(c) => format!("s3:{}:{}:{path}", c.endpoint, c.bucket),
        };
        Ok(crate::capture::sha256(identity.as_bytes()))
    }

    pub fn copies(&self, inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let build = self.root.join(format!("build-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&build)?;
        inputs
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let destination = build.join(i.to_string());
                copy_directory(input, &destination)?;
                Ok(destination)
            })
            .collect()
    }

    pub fn publish(&self, staged: &Path) -> Result<()> {
        anyhow::ensure!(
            staged.starts_with(&self.root) && !staged.components().any(|part| part == std::path::Component::ParentDir) && staged.is_dir(),
            "publication must come from the consumer workspace"
        );
        sync_directory(staged)?;
        let publication = Publication {
            version: 1,
            staged: staged.into(),
            backup: self.root.join(format!("previous-{}", uuid::Uuid::new_v4())),
            complete: false,
        };
        super::consumer::atomic_json(&self.root.join("publication.json"), &publication)?;
        self.recover()
    }

    fn recover(&self) -> Result<()> {
        let journal = self.root.join("publication.json");
        if !journal.exists() {
            return Ok(());
        }
        let mut publication: Publication =
            serde_json::from_reader(File::open(&journal)?.take(65536))?;
        anyhow::ensure!(
            publication.version == 1
                && publication.staged.starts_with(&self.root)
                && publication.backup.starts_with(&self.root)
                && !publication.staged.components().chain(publication.backup.components()).any(|part| part == std::path::Component::ParentDir),
            "invalid consumer publication journal"
        );
        if publication.complete {
            return Ok(());
        }
        if publication.staged.exists() {
            if self.output.exists() {
                anyhow::ensure!(
                    !publication.backup.exists(),
                    "ambiguous consumer publication: both output and backup exist"
                );
                fs::rename(&self.output, &publication.backup)?;
                File::open(&self.root)?.sync_all()?;
            }
            fs::rename(&publication.staged, &self.output)?;
        } else if !self.output.exists() {
            if publication.backup.exists() {
                fs::rename(&publication.backup, &self.output)?;
            }
            anyhow::bail!("staged consumer output missing; previous output restored if available");
        }
        File::open(
            self.output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
        publication.complete = true;
        super::consumer::atomic_json(&journal, &publication)
    }
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
            File::open(target)?.sync_all()?;
        } else {
            anyhow::bail!("unsupported special file in consumer output");
        }
    }
    File::open(destination)?.sync_all()?;
    Ok(())
}

fn sync_directory(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_directory(&entry.path())?;
        } else if entry.file_type()?.is_file() {
            File::open(entry.path())?.sync_all()?;
        } else {
            anyhow::bail!("unsupported special file in staged output");
        }
    }
    File::open(directory)?.sync_all()?;
    Ok(())
}
