use std::{fs, path::PathBuf};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
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
