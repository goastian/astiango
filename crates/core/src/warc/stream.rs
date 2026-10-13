use std::{fs::File, io::Read, path::Path, time::Duration};

use crate::{config::WarcSource, Result};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub record_bytes: usize,
    pub decoded_bytes: usize,
    pub header_bytes: usize,
    pub batch_bytes: usize,
    pub workers: usize,
    pub stack_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            record_bytes: 40 * 1024 * 1024,
            decoded_bytes: 32 * 1024 * 1024,
            header_bytes: 64 * 1024,
            batch_bytes: 64 * 1024 * 1024,
            workers: 2,
            stack_bytes: 8 * 1024 * 1024,
        }
    }
}

impl Limits {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.record_bytes > 0 && self.decoded_bytes > 0,
            "consumer byte limits must be positive"
        );
        anyhow::ensure!(
            self.batch_bytes >= self.decoded_bytes,
            "a decoded record must fit the consumer batch budget"
        );
        anyhow::ensure!(
            (1024..=1024 * 1024).contains(&self.header_bytes),
            "consumer header limit must be 1 KiB..1 MiB"
        );
        anyhow::ensure!(
            (1..=32).contains(&self.workers)
                && (1024 * 1024..=64 * 1024 * 1024).contains(&self.stack_bytes),
            "invalid consumer pool or stack limit"
        );
        Ok(())
    }
}

pub fn open(
    source: &WarcSource,
    path: &str,
    limits: Limits,
) -> Result<super::RecordIterator<Box<dyn Read>>> {
    limits.validate()?;
    Ok(super::RecordIterator::with_limits(
        open_reader(source, path)?,
        limits,
    ))
}

pub fn open_reader(source: &WarcSource, path: &str) -> Result<Box<dyn Read>> {
    let reader: Box<dyn Read> = match source {
        WarcSource::Local(config) => Box::new(File::open(Path::new(&config.folder).join(path))?),
        WarcSource::HTTP(config) => {
            let url = format!(
                "{}/{}",
                config.base_url.trim_end_matches('/'),
                path.trim_start_matches('/')
            );
            Box::new(client()?.get(url).send()?.error_for_status()?)
        }
        WarcSource::S3(config) => {
            let bucket = s3::Bucket::new(
                &config.bucket,
                s3::Region::Custom {
                    region: String::new(),
                    endpoint: config.endpoint.clone(),
                },
                s3::creds::Credentials {
                    access_key: Some(config.access_key.clone()),
                    secret_key: Some(config.secret_key.clone()),
                    security_token: None,
                    session_token: None,
                    expiration: None,
                },
            )?
            .with_path_style();
            let url = crate::block_on(bucket.presign_get(path, 3600, None))?;
            Box::new(client()?.get(url).send()?.error_for_status()?)
        }
    };
    Ok(reader)
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(1800))
        .build()?)
}
