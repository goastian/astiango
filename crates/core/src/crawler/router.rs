use anyhow::Result;
use rand::Rng;
use std::{net::SocketAddr, time::Duration};
use tokio::sync::Mutex;

use crate::{
    distributed::{retry_strategy::ExponentialBackoff, sonic},
    entrypoint::crawler::coordinator::{CoordinatorService, GetJob},
};

use super::Job;

#[derive(Clone)]
struct RemoteCoordinator {
    addr: SocketAddr,
}

impl RemoteCoordinator {
    async fn conn(&self) -> Result<sonic::service::Connection<CoordinatorService>> {
        let retry = ExponentialBackoff::from_millis(1_000).with_limit(Duration::from_secs(10));

        Ok(sonic::service::Connection::create_with_timeout_retry(
            self.addr,
            Duration::from_secs(60),
            retry,
        )
        .await?)
    }

    async fn sample_job(&self) -> Result<Option<Job>> {
        let mut conn = self.conn().await?;

        let response = conn
            .send_with_timeout(GetJob {}, Duration::from_secs(90))
            .await?;

        Ok(response)
    }
}

struct InnerRouter {
    coordinators: Vec<RemoteCoordinator>,
}

impl InnerRouter {
    async fn new(coordinator_addrs: Vec<SocketAddr>) -> Result<Self> {
        Ok(Self {
            coordinators: coordinator_addrs
                .into_iter()
                .map(|addr| RemoteCoordinator { addr })
                .collect(),
        })
    }

}

pub struct Router {
    inner: Mutex<InnerRouter>,
}

impl Router {
    pub async fn new(coordinator_addrs: Vec<SocketAddr>) -> Result<Self> {
        Ok(Self {
            inner: Mutex::new(InnerRouter::new(coordinator_addrs).await?),
        })
    }

    pub async fn sample_job(&self) -> Result<Option<Job>> {
        loop {
            let selected = {
                let inner = self.inner.lock().await;
                if inner.coordinators.is_empty() { return Ok(None); }
                inner.coordinators[rand::thread_rng().gen_range(0..inner.coordinators.len())].clone()
            };
            // Network errors propagate; only a confirmed empty queue retires a peer.
            let result = selected.sample_job().await?;
            if result.is_some() { return Ok(result); }
            self.inner.lock().await.coordinators.retain(|peer| peer.addr != selected.addr);
        }
    }
}
