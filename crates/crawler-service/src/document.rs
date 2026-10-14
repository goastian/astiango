//! A single bounded public PDF capture for specialized, opt-in acquisition.
use anyhow::{Context, Result};
use astiango_crawler_service as astiango;
use std::{io::Write, path::Path, sync::Arc, time::Duration};
pub async fn capture(config: astiango::crawler::local::Config, url: url::Url, output: &Path) -> Result<()> {
    config.validate()?;
    let settings=config.crawler_config();
    astiango::crawler::robot_client::validate_seed_urls(&settings, &[url.clone()])?;
    let client=astiango::crawler::robot_client::RobotClient::new(&settings)?;
    let operation=async {
        let started=std::time::Instant::now();
        let mut response=client.get(url.clone()).await?.timeout(Duration::from_secs(10)).send().await?;
        anyhow::ensure!(response.status()==reqwest::StatusCode::OK,"PDF admission rejected HTTP {}",response.status());
        let metadata=astiango::capture::HttpResponse::received(&response,&url,chrono::Utc::now());
        anyhow::ensure!(metadata.values("content-type").any(|v|v.split(';').next().is_some_and(|m|m.trim().eq_ignore_ascii_case("application/pdf"))),"PDF MIME rejected before body");
        anyhow::ensure!(metadata.values("content-encoding").all(|v|v.eq_ignore_ascii_case("identity")),"unsupported encoded PDF capture");
        anyhow::ensure!(response.content_length().is_none_or(|n|n<=4*1024*1024),"PDF length rejected before body");
        let mut payload=Vec::new();
        let body=async {
            while let Some(chunk)=tokio::time::timeout(Duration::from_secs(2),response.chunk()).await?? {
                anyhow::ensure!(payload.len()+chunk.len()<=4*1024*1024,"PDF wire quota reached");payload.extend_from_slice(&chunk);
            }
            Ok::<_,anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(10),body).await??;
        anyhow::ensure!(payload.starts_with(b"%PDF-"),"PDF signature rejected");
        let capture=astiango::capture::Capture {metadata:astiango::capture::Metadata {
            schema_version:1,identity_version:1,fetch_id:uuid::Uuid::new_v4().to_string(),requested_url:astiango::capture::url_key(&url)?,
            payload_sha256:astiango::capture::sha256(&payload),payload_bytes:payload.len(),decoded_bytes:0,decoded_limit_bytes:4*1024*1024,
            charset:"utf-8".into(),user_agent:settings.user_agent.token.clone(),robots:astiango::capture::Robots::from_headers(&metadata,&settings.user_agent.token),
            transformations:metadata.transformations(),response:metadata,redirects:Vec::new(),fetch_time_ms:started.elapsed().as_millis() as u64,
            discovery_urls:Vec::new(),internal_urls:Vec::new(),revalidation:None},payload:Arc::new(payload)};
        capture.metadata.validate()?;
        std::fs::create_dir_all(output)?;
        for (name,bytes) in [("original.pdf",capture.payload.as_slice().to_vec()),("capture.json",serde_json::to_vec_pretty(&capture.metadata)?)] {
            let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(output.join(name))?;file.write_all(&bytes)?;file.sync_all()?;
        }
        std::fs::File::open(output)?.sync_all()?;
        println!("{}",serde_json::json!({"captured":true,"fetch_id":capture.metadata.fetch_id,"bytes":capture.metadata.payload_bytes,"public_destination_validated":capture.metadata.response.address_validated,"output":output}));
        Ok::<_,anyhow::Error>(())
    };
    let result=tokio::time::timeout(Duration::from_secs(45),operation).await.context("specialized capture total timeout")?;
    client.shutdown().await;result
}
