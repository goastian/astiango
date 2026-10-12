use std::{fs, path::PathBuf};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--inspect-identities" {
        use redb::ReadableTable;
        use ring::digest::{Context, SHA256};
        let db = redb::Database::builder()
            .set_cache_size(16 * 1024 * 1024)
            .open(&args[2])?;
        let tx = db.begin_read()?;
        let meta = tx.open_table(redb::TableDefinition::<&str, &[u8]>::new("metadata.v1"))?;
        let stats: serde_json::Value = serde_json::from_slice(meta.get("stats")?.unwrap().value())?;
        let urls = tx.open_table(redb::TableDefinition::<&str, &[u8]>::new("urls.v1"))?;
        let (mut total, mut unfinished, mut committed) = (0u64, 0u64, 0u64);
        let mut digest = Context::new(&SHA256);
        for entry in urls.iter()? {
            let (key, value) = entry?;
            let row: serde_json::Value = serde_json::from_slice(value.value())?;
            digest.update(key.value().as_bytes());
            digest.update(&[0]);
            digest.update(row["fetch_id"].to_string().as_bytes());
            digest.update(&[0]);
            let state = row["state"].as_str().unwrap();
            let state = if matches!(state, "Pending" | "Leased") {
                unfinished += 1;
                "Unfinished"
            } else {
                state
            };
            digest.update(state.as_bytes());
            digest.update(&[0]);
            committed += u64::from(state == "Committed");
            total += 1;
        }
        let site_cursor = meta
            .get("site-migration-cursor")?
            .map(|v| serde_json::from_slice::<String>(v.value()))
            .transpose()?;
        println!(
            "{}",
            serde_json::json!({"passed":true,"frontier":stats,"urls":total,"unfinished":unfinished,"committed":committed,
            "url_fetch_identity_sha256":astiango::capture::hex(digest.finish().as_ref()),"site_migration_cursor":site_cursor})
        );
        return Ok(());
    }
    if args.len() == 3 && args[1] == "--inspect-frontier" {
        let db = redb::Database::builder()
            .set_cache_size(16 * 1024 * 1024)
            .open(&args[2])?;
        let tx = db.begin_read()?;
        let table = tx.open_table(redb::TableDefinition::<&str, &[u8]>::new("metadata.v1"))?;
        let value = table
            .get("stats")?
            .ok_or_else(|| anyhow::anyhow!("frontier statistics missing"))?;
        let stats: serde_json::Value = serde_json::from_slice(value.value())?;
        println!("{}", serde_json::json!({"passed":true,"frontier":stats}));
        return Ok(());
    }
    if args.len() == 3 && args[1] == "--frontier" {
        let frontier = astiango::crawler::frontier::Frontier::open_database(
            PathBuf::from(&args[2]),
            256,
            10000,
        )?;
        println!(
            "{}",
            serde_json::json!({"passed":true,"frontier":frontier.metrics.snapshot()})
        );
        return Ok(());
    }
    anyhow::ensure!(
        args.len() == 3 && args[1] == "--directory",
        "usage: crawler_archive_contract --directory PATH"
    );
    let mut stack = vec![PathBuf::from(&args[2])];
    let (mut files, mut records, mut legacy, mut current) = (0u64, 0u64, 0u64, 0u64);
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                stack.push(entry?.path());
            }
            continue;
        }
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".warc.gz"))
        {
            continue;
        }
        files += 1;
        for record in astiango::warc::RecordIterator::new(fs::File::open(&path)?) {
            let record = record?;
            if record.metadata.capture.is_some() {
                current += 1;
            } else {
                legacy += 1;
            }
            records += 1;
        }
    }
    anyhow::ensure!(files > 0 && records > 0, "archive contract corpus is empty");
    println!(
        "{}",
        serde_json::json!({"passed":true,"files":files,"records":records,"legacy_records":legacy,"capture_v1_records":current})
    );
    Ok(())
}
