//! Immutable, validated projections of the exact ingestion ledger. The working index
//! retains history; serving opens one selected generation, never that working index.
use super::*;
use tantivy::schema::{OwnedValue, TantivyDocument, Value};

pub(super) const PREPARED: TableDefinition<&str, &[u8]> = TableDefinition::new("prepared.v1");
pub(super) const INPUTS: TableDefinition<&str, &[u8]> = TableDefinition::new("inputs.v1");
pub(super) const CONTEXTS: TableDefinition<&str, &[u8]> =
    TableDefinition::new("preparation-contexts.v1");
pub(super) const PREPARED_CONTEXTS: TableDefinition<&str, &[u8]> =
    TableDefinition::new("prepared-contexts.v1");

pub(super) fn initialize_context(ledger: &Ledger, base: &IndexerConfig) -> Result<()> {
    let mut config = redact(serde_json::to_value(base)?);
    for key in [
        "warc_source",
        "output_path",
        "limit_warc_files",
        "skip_warc_files",
        "consumer",
        "dedup",
        "batch_size",
        "autocommit_after_num_inserts",
    ] {
        config.as_object_mut().unwrap().remove(key);
    }
    let artifacts = if matches!(
        base.page_webgraph,
        Some(crate::config::IndexerGraphConfig::Remote { .. })
    ) {
        serde_json::json!({"remote_graph":"not pinned; generation requires local snapshot"})
    } else {
        signals(base)?
    };
    let content =
        serde_json::json!({"version":1,"config":config,"artifacts":artifacts,"build":build_info()});
    let id = crate::capture::sha256(&serde_json::to_vec(&content)?);
    let bytes = serde_json::to_vec(&serde_json::json!({"id":id,"context":content}))?;
    let tx = ledger.db.begin_write()?;
    {
        let mut table = tx.open_table(CONTEXTS)?;
        table.insert(id.as_str(), bytes.as_slice())?;
        table.insert("current", bytes.as_slice())?;
    }
    tx.commit()?;
    Ok(())
}
const BUCKETS: usize = 16;
const VERSION: u32 = 1;

// Tagged values preserve bytes, dates, pre-tokenized strings and u128 values.
// Ordinary schema JSON loses these distinctions and cannot replay a prepared document.
#[derive(Serialize, Deserialize)]
enum FieldValue {
    Null,
    Str(String),
    Tokens(tantivy::tokenizer::PreTokenizedString),
    U64(u64),
    U128(String),
    I64(i64),
    F64(f64),
    Bool(bool),
    Date(i64),
    Bytes(Vec<u8>),
    Ip(std::net::Ipv6Addr),
    Array(Vec<FieldValue>),
    Object(Vec<(String, FieldValue)>),
}
impl FieldValue {
    fn encode(value: OwnedValue) -> Self {
        match value {
            OwnedValue::Null => Self::Null,
            OwnedValue::Str(v) => Self::Str(v),
            OwnedValue::PreTokStr(v) => Self::Tokens(v),
            OwnedValue::U64(v) => Self::U64(v),
            OwnedValue::U128(v) => Self::U128(v.to_string()),
            OwnedValue::I64(v) => Self::I64(v),
            OwnedValue::F64(v) => Self::F64(v),
            OwnedValue::Bool(v) => Self::Bool(v),
            OwnedValue::Date(v) => Self::Date(v.into_timestamp_nanos()),
            OwnedValue::Bytes(v) => Self::Bytes(v),
            OwnedValue::IpAddr(v) => Self::Ip(v),
            OwnedValue::Array(v) => Self::Array(v.into_iter().map(Self::encode).collect()),
            OwnedValue::Object(v) => {
                Self::Object(v.into_iter().map(|(k, v)| (k, Self::encode(v))).collect())
            }
        }
    }
    fn decode(self) -> Result<OwnedValue> {
        Ok(match self {
            Self::Null => OwnedValue::Null,
            Self::Str(v) => OwnedValue::Str(v),
            Self::Tokens(v) => OwnedValue::PreTokStr(v),
            Self::U64(v) => OwnedValue::U64(v),
            Self::U128(v) => OwnedValue::U128(v.parse()?),
            Self::I64(v) => OwnedValue::I64(v),
            Self::F64(v) => {
                anyhow::ensure!(v.is_finite(), "invalid prepared numeric value");
                OwnedValue::F64(v)
            }
            Self::Bool(v) => OwnedValue::Bool(v),
            Self::Date(v) => OwnedValue::Date(tantivy::DateTime::from_timestamp_nanos(v)),
            Self::Bytes(v) => OwnedValue::Bytes(v),
            Self::Ip(v) => OwnedValue::IpAddr(v),
            Self::Array(v) => {
                OwnedValue::Array(v.into_iter().map(Self::decode).collect::<Result<_>>()?)
            }
            Self::Object(v) => OwnedValue::Object(
                v.into_iter()
                    .map(|(k, v)| Ok((k, v.decode()?)))
                    .collect::<Result<_>>()?,
            ),
        })
    }
}
#[derive(Serialize, Deserialize)]
struct Prepared {
    version: u32,
    schema: String,
    document_identity: String,
    fields: Vec<(u32, FieldValue)>,
}
fn redact(mut value: serde_json::Value) -> serde_json::Value {
    match &mut value {
        serde_json::Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if [
                    "access_key",
                    "secret_key",
                    "password",
                    "token",
                    "security_token",
                ]
                .contains(&key.as_str())
                {
                    *item = "[redacted]".into();
                } else {
                    *item = redact(item.take());
                }
            }
        }
        serde_json::Value::Array(array) => {
            for item in array {
                *item = redact(item.take());
            }
        }
        _ => {}
    }
    value
}
fn schema_hash() -> Result<String> {
    Ok(crate::capture::sha256(&serde_json::to_vec(
        &crate::schema::create_schema(),
    )?))
}
pub(super) fn save_prepared(ledger: &Ledger, row: &Decision, doc: &TantivyDocument) -> Result<()> {
    let prepared = Prepared {
        version: VERSION,
        schema: schema_hash()?,
        document_identity: row
            .indexed_document_sha256
            .clone()
            .context("missing prepared identity")?,
        fields: doc
            .field_values()
            .map(|(f, v)| {
                (
                    f.field_id(),
                    FieldValue::encode(OwnedValue::from(v.as_value())),
                )
            })
            .collect(),
    };
    struct Limited<W> {
        inner: W,
        written: usize,
    }
    impl<W: Write> Write for Limited<W> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.written.saturating_add(bytes.len()) > 128 * 1024 * 1024 {
                return Err(std::io::Error::other(
                    "prepared replay exceeds per-document quota",
                ));
            }
            let n = self.inner.write(bytes)?;
            self.written += n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }
    let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut output = Limited {
        inner: std::io::BufWriter::with_capacity(64 * 1024, gzip),
        written: 0,
    };
    output.write_all(b"ASTPRD2\n")?;
    bincode::serde::encode_into_std_write(&prepared, &mut output, bincode::config::standard())?;
    let bytes = output
        .inner
        .into_inner()
        .map_err(|e| anyhow::anyhow!("prepared buffer flush: {e}"))?
        .finish()?;
    let mut tx = ledger.db.begin_write()?;
    // This cache is reconstructible from retained captures. The subsequent business
    // ledger commit also flushes it; no independent per-document durable ACK is issued.
    tx.set_durability(redb::Durability::None);
    let context = ledger
        .db
        .begin_read()?
        .open_table(CONTEXTS)?
        .get("current")?
        .context("preparation context missing")?
        .value()
        .to_vec();
    {
        tx.open_table(PREPARED_CONTEXTS)?
            .insert(row.version.as_str(), context.as_slice())?;
    }
    {
        tx.open_table(PREPARED)?
            .insert(row.version.as_str(), bytes.as_slice())?;
    }
    tx.commit()?;
    Ok(())
}
fn decode_prepared(
    bytes: &[u8],
    signal: Option<(&crate::webgraph::signals::Bundle, &str)>,
) -> Result<TantivyDocument> {
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut decoded)?;
    anyhow::ensure!(
        decoded.len() <= 128 * 1024 * 1024,
        "prepared replay expansion exceeds quota"
    );
    let prepared: Prepared = if decoded.starts_with(b"ASTPRD2\n") {
        let (value, consumed) = bincode::serde::decode_from_slice(
            &decoded[8..],
            bincode::config::standard().with_limit::<{ 128 * 1024 * 1024 }>(),
        )?;
        anyhow::ensure!(
            consumed == decoded.len() - 8,
            "trailing prepared replay data"
        );
        value
    } else {
        anyhow::ensure!(
            !decoded.starts_with(b"ASTPRD"),
            "unsupported prepared replay codec"
        );
        serde_json::from_slice(&decoded)?
    };
    anyhow::ensure!(
        prepared.version == VERSION && prepared.schema == schema_hash()?,
        "incompatible prepared document/schema version"
    );
    let mut doc = TantivyDocument::new();
    let schema = crate::schema::create_schema();
    let overlay = signal
        .map(|(bundle, url)| -> Result<_> {
            let prefix = if bundle.scope == "page" {
                "page"
            } else {
                "host"
            };
            Ok((
                schema.get_field(&format!("{prefix}_centrality"))?,
                schema.get_field(&format!("{prefix}_centrality_rank"))?,
                bundle.lookup(&url::Url::parse(url)?)?,
            ))
        })
        .transpose()?;
    let count = schema.num_fields() as u32;
    for (field, value) in prepared.fields {
        anyhow::ensure!(field < count, "unknown prepared field");
        if overlay
            .as_ref()
            .is_some_and(|(score, rank, _)| field == score.field_id() || field == rank.field_id())
        {
            continue;
        }
        doc.add_field_value(
            tantivy::schema::Field::from_field_id(field),
            &value.decode()?,
        );
    }
    if let Some((score, rank, (value, ordinal))) = overlay {
        doc.add_f64(score, value);
        doc.add_u64(rank, ordinal);
    }
    Ok(doc)
}
pub(super) fn save_input(
    ledger: &Ledger,
    key: &str,
    source: &crate::config::WarcSource,
    name: &str,
) -> Result<()> {
    let value = serde_json::to_vec(
        &serde_json::json!({"version":1,"source":redact(serde_json::to_value(source)?),"name":name,"sha256":key.rsplit(':').next()}),
    )?;
    let tx = ledger.db.begin_write()?;
    {
        tx.open_table(INPUTS)?.insert(key, value.as_slice())?;
    }
    tx.commit()?;
    Ok(())
}
fn lock_root(root: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("ingestion.lock"))?;
    lock.try_lock_exclusive()
        .context("ingestion busy; retry generation operation")?;
    Ok(lock)
}
fn json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    anyhow::ensure!(
        fs::metadata(path)?.len() <= limit,
        "generation metadata exceeds budget"
    );
    Ok(serde_json::from_reader(File::open(path)?.take(limit))?)
}
fn atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::warc::consumer::atomic_json(path, value)
}
fn bucket(url: &str) -> usize {
    usize::from_str_radix(&crate::capture::sha256(url.as_bytes())[..1], 16).unwrap()
}
fn checksum(path: &Path) -> Result<String> {
    crate::warc::consumer::checksum(File::open(path)?)
}
fn stop_check() -> Result<()> {
    anyhow::ensure!(
        !CANCELLED.load(std::sync::atomic::Ordering::Relaxed),
        "cancelled: generation retained; active selector unchanged"
    );
    Ok(())
}
fn fail(name: &str) {
    super::failpoint(name);
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub state: String,
    pub created_at: String,
    pub schema_sha256: String,
    pub extractor: String,
    pub engine: String,
    pub build: serde_json::Value,
    pub config: serde_json::Value,
    pub signals: serde_json::Value,
    pub documents: u64,
    pub outcomes: BTreeMap<String, u64>,
    pub pending: u64,
    pub logical_sha256: String,
    pub files: BTreeMap<String, String>,
    pub bucket_hashes: Vec<String>,
    pub bucket_segments: Vec<Vec<String>>,
    pub elapsed_ms: f64,
    pub reused_buckets: usize,
    pub rebuilt_buckets: usize,
    pub validation: serde_json::Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    pub version: u32,
    pub sequence: u64,
    pub active: String,
    pub previous: Option<String>,
    pub manifest_sha256: String,
}
fn id_path(root: &Path, id: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        id.len() == 36 && uuid::Uuid::parse_str(id).is_ok(),
        "invalid generation ID"
    );
    Ok(root.join("generations").join(id))
}
fn selector(root: &Path) -> Result<Option<Selector>> {
    let path = root.join("generation-selector.json");
    if !path.exists() {
        return Ok(None);
    }
    let value: Selector = json(&path, 65536)?;
    anyhow::ensure!(
        value.version == VERSION,
        "unknown generation selector version {}",
        value.version
    );
    id_path(root, &value.active)?;
    if let Some(id) = &value.previous {
        id_path(root, id)?;
    }
    Ok(Some(value))
}
fn validate_manifest(root: &Path, id: &str, full: bool) -> Result<Manifest> {
    let directory = id_path(root, id)?;
    let manifest: Manifest = json(&directory.join("manifest.json"), 1024 * 1024)?;
    anyhow::ensure!(
        manifest.version == VERSION,
        "unsupported generation manifest version {}",
        manifest.version
    );
    anyhow::ensure!(
        manifest.id == id
            && manifest.state == "ready"
            && manifest.schema_sha256 == schema_hash()?
            && manifest.bucket_hashes.len() == BUCKETS
            && manifest.bucket_segments.len() == BUCKETS,
        "generation is not ready or schema incompatible"
    );
    anyhow::ensure!(
        manifest.validation["passed"] == true,
        "generation content validation missing"
    );
    if full {
        for (name, hash) in &manifest.files {
            let path = Path::new(name);
            anyhow::ensure!(
                !path.is_absolute()
                    && path
                        .components()
                        .all(|p| matches!(p, std::path::Component::Normal(_))),
                "invalid generation artifact path"
            );
            anyhow::ensure!(
                checksum(&directory.join(path))? == *hash,
                "generation checksum mismatch: {name}"
            );
        }
        let index = Index::open(&directory)?;
        anyhow::ensure!(
            index.inverted_index.num_documents() == manifest.documents,
            "generation engine count mismatch"
        );
    }
    Ok(manifest)
}
/// Existing search_server can point index_path at the ingestion root. Resolve one
/// validated generation at load; changing the selector never mutates a live reader.
pub fn serving_path(path: &Path) -> Result<PathBuf> {
    if let Some(selected) = selector(path)? {
        let directory = id_path(path, &selected.active)?;
        anyhow::ensure!(
            checksum(&directory.join("manifest.json"))? == selected.manifest_sha256,
            "selected manifest checksum mismatch"
        );
        validate_manifest(path, &selected.active, true)?;
        Ok(directory)
    } else {
        anyhow::ensure!(
            !path.join("ledger.redb").exists(),
            "shared working index has no validated serving selector"
        );
        anyhow::ensure!(
            path.join("inverted_index/meta.json").is_file(),
            "serving index does not exist"
        );
        Ok(path.to_path_buf())
    }
}

// Backfill only missing prepared versions, with existing exact URL/content checked.
// This migrates the historical local ledger without changing decisions or IDs.
fn backfill(
    base: &IndexerConfig,
    policy: &Config,
    ledger: &Ledger,
    folder: Option<&Path>,
) -> Result<u64> {
    let tx = ledger.db.begin_read()?;
    let current = tx.open_table(CURRENT)?;
    let mut missing = 0;
    let mut needed_sources = std::collections::BTreeSet::new();
    for item in current.iter()? {
        let (_, v) = item?;
        let row: Decision = serde_json::from_slice(v.value())?;
        if row.outcome == "indexed"
            && tx
                .open_table(PREPARED)?
                .get(row.version.as_str())?
                .is_none()
        {
            missing += 1;
            needed_sources.insert(row.source);
            anyhow::ensure!(
                needed_sources.len() <= 4096,
                "historical source index exceeds budget"
            );
        }
    }
    drop(current);
    drop(tx);
    if missing == 0 {
        return Ok(0);
    }
    let mut source = base.warc_source.clone();
    if let Some(folder) = folder {
        let crate::config::WarcSource::Local(local) = &mut source else {
            anyhow::bail!("backfill directory requires local sources");
        };
        local.folder = folder.to_string_lossy().into();
        let mut names = Vec::new();
        for file in fs::read_dir(folder)? {
            let file = file?;
            if file.file_type()?.is_file()
                && file.file_name().to_string_lossy().ends_with(".warc.gz")
            {
                anyhow::ensure!(names.len() < 4096, "backfill source scan exceeds quota");
                names.push(file.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
        local.names = names;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut worker = rt.block_on(IndexingWorker::try_new(base.clone().into()))?;
    worker.set_job_settings(JobSettings {
        host_centrality_threshold: base.host_centrality_threshold,
        minimum_clean_words: base.minimum_clean_words,
        batch_size: base.batch_size,
        autocommit_after_num_inserts: policy.commit_records,
    });
    let mut cache = TextCache::new(policy.cache_bytes);
    let index = Index::open(Path::new(&base.output_path).join("working"))?;
    let mut restored = 0;
    // Reference inputs use byte ranges rather than an invented ordinal. Recover the
    // same capture if its reconstructible prepared cache ever needs rebuilding.
    {
        let read = ledger.db.begin_read()?;
        let rows = read.open_table(CURRENT)?;
        for entry in rows.iter()? {
            let (_, value) = entry?;
            let row: Decision = serde_json::from_slice(value.value())?;
            if row.outcome != "indexed"
                || read
                    .open_table(PREPARED)?
                    .get(row.version.as_str())?
                    .is_some()
            {
                continue;
            }
            if let Some(reference) = &row.source_reference {
                let (record, _, _) = super::read_reference(&source, reference, base.consumer)?;
                let page =
                    rt.block_on(worker.prepare_one(&IndexableWebpage::from(record), &mut cache))?;
                let hash = super::document_identity(
                    &row.url,
                    &page.html.title().unwrap_or_default(),
                    page.html.clean_text().map_or("", String::as_str),
                );
                anyhow::ensure!(
                    row.indexed_document_sha256.as_ref() == Some(&hash),
                    "referenced replay changed document identity"
                );
                save_prepared(ledger, &row, &index.inverted_index.prepare_document(&page)?)?;
                restored += 1;
            }
        }
    }

    for name in source.paths()? {
        stop_check()?;
        let key = crate::warc::publication::Workspace::key(&source, &name)?;
        if !needed_sources
            .iter()
            .any(|id| id.starts_with(&format!("{key}:")))
        {
            continue;
        }
        let reader = crate::warc::stream::open(&source, &name, base.consumer)?;
        let fingerprint=match &source { crate::config::WarcSource::Local(local)=>checksum(&Path::new(&local.folder).join(&name))?, _=>anyhow::bail!("historical backfill requires locally retained source; new ingestion records prepared values directly") };
        let source_id = format!("{key}:{fingerprint}");
        save_input(ledger, &source_id, &source, &name)?;
        for (ordinal, record) in reader.enumerate() {
            stop_check()?;
            let record = record?;
            let url = crate::capture::url_key(&url::Url::parse(&record.request.url)?)?;
            let Some(row) = ledger.current(&url)? else {
                continue;
            };
            if row.outcome != "indexed"
                || row.source != source_id
                || row.record != ordinal as u64 + 1
            {
                continue;
            }
            if ledger
                .db
                .begin_read()?
                .open_table(PREPARED)?
                .get(row.version.as_str())?
                .is_some()
            {
                continue;
            }
            let page =
                rt.block_on(worker.prepare_one(&IndexableWebpage::from(record), &mut cache))?;
            let identity = super::document_identity(
                &url,
                &page.html.title().unwrap_or_default(),
                page.html.clean_text().map_or("", String::as_str),
            );
            anyhow::ensure!(
                row.indexed_document_sha256
                    .as_ref()
                    .is_none_or(|hash| hash == &identity),
                "backfill changed indexed content; retain previous generation"
            );
            let mut row = row;
            row.indexed_document_sha256 = Some(identity);
            save_prepared(ledger, &row, &index.inverted_index.prepare_document(&page)?)?;
            // Older prototype rows gain a checked identity without altering IDs/versions.
            let tx = ledger.db.begin_write()?;
            {
                let bytes = serde_json::to_vec(&row)?;
                tx.open_table(CURRENT)?
                    .insert(row.url.as_str(), bytes.as_slice())?;
            }
            tx.commit()?;
            restored += 1;
        }
        if restored == missing {
            break;
        }
    }
    anyhow::ensure!(restored==missing,"generation partial: {}/{} historical documents replayed; retain captures and provide all source paths/derivatives",restored,missing);
    Ok(restored)
}
fn inventory(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut output = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for file in fs::read_dir(&dir)? {
            let file = file?;
            let kind = file.file_type()?;
            anyhow::ensure!(!kind.is_symlink(), "generation contains a symlink");
            if kind.is_dir() {
                stack.push(file.path());
            } else if kind.is_file() {
                let name = file
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .into_owned();
                if name == "manifest.json" || name.ends_with(".lock") || name == ".lock" {
                    continue;
                }
                anyhow::ensure!(output.len() < 100000, "generation inventory exceeds quota");
                output.insert(name, checksum(&file.path())?);
            } else {
                anyhow::bail!("unsupported generation artifact");
            }
        }
    }
    Ok(output)
}
fn clone_index(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to.join("inverted_index"))?;
    for file in fs::read_dir(from.join("inverted_index"))? {
        let file = file?;
        if !file.file_type()?.is_file() {
            continue;
        }
        let name = file.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".lock") {
            continue;
        }
        let dest = to.join("inverted_index").join(name.as_ref());
        if name.starts_with('.') || name == "meta.json" {
            fs::copy(file.path(), dest)?;
        } else {
            fs::hard_link(file.path(), dest)?;
        }
    }
    if from.join("region_count.json").exists() {
        fs::copy(from.join("region_count.json"), to.join("region_count.json"))?;
    }
    Ok(())
}
fn segments(directory: &Path) -> Result<Vec<String>> {
    let value: serde_json::Value =
        json(&directory.join("inverted_index/meta.json"), 4 * 1024 * 1024)?;
    Ok(value["segments"]
        .as_array()
        .context("missing engine segments")?
        .iter()
        .map(|s| {
            s["segment_id"]
                .as_str()
                .context("missing segment ID")
                .map(str::to_owned)
        })
        .collect::<Result<_>>()?)
}
fn sync_tree(root: &Path) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = Vec::new();
    while let Some(dir) = stack.pop() {
        dirs.push(dir.clone());
        for f in fs::read_dir(&dir)? {
            let f = f?;
            if f.file_type()?.is_dir() {
                stack.push(f.path());
            } else if f.file_type()?.is_file() {
                File::open(f.path())?.sync_all()?;
            }
        }
    }
    for dir in dirs.into_iter().rev() {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}
fn signals(base: &IndexerConfig) -> Result<serde_json::Value> {
    let mut hashes = BTreeMap::new();
    for path in [
        Some(base.host_centrality_store_path.as_str()),
        base.page_centrality_store_path.as_deref(),
        base.safety_classifier_path.as_deref(),
        base.dual_encoder.as_ref().map(|v| v.model_path.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let root = Path::new(path);
        if root.is_file() {
            hashes.insert(path.to_string(), checksum(root)?);
        } else if root.is_dir() {
            hashes.insert(
                path.to_string(),
                crate::capture::sha256(&serde_json::to_vec(&inventory(root)?)?),
            );
        } else {
            anyhow::bail!("configured signal artifact unavailable: {path}");
        }
    }
    if let Some(crate::config::IndexerGraphConfig::Local { path }) = &base.page_webgraph {
        hashes.insert(
            path.clone(),
            crate::capture::sha256(&serde_json::to_vec(&inventory(Path::new(path))?)?),
        );
    }
    anyhow::ensure!(
        !matches!(
            base.page_webgraph,
            Some(crate::config::IndexerGraphConfig::Remote { .. })
        ),
        "reproducible generation requires a pinned local graph snapshot"
    );
    Ok(
        serde_json::json!({"artifacts":hashes,"models_configured":base.dual_encoder.is_some()||base.safety_classifier_path.is_some(),"graph_configured":base.page_webgraph.is_some(),"prepared_values":"preserved; missing historical values reconstructed from pinned configuration"}),
    )
}
pub fn build_info() -> serde_json::Value {
    serde_json::json!({"version":1,"build_id":option_env!("INDEXER_BUILD_ID").unwrap_or("unpackaged"),"commit":option_env!("INDEXER_GIT_COMMIT").unwrap_or("unknown"),"rust":option_env!("INDEXER_RUST_VERSION").unwrap_or("1.90.0"),"lock_sha256":option_env!("INDEXER_LOCK_SHA256").unwrap_or("unknown"),"engine":"tantivy-fork-0.23/schema-v1","manifest_versions":[1],"legacy_index_reader":true,"prepared_cache_codecs":[1,2]})
}

pub fn prepare(
    base: &IndexerConfig,
    policy: &Config,
    folder: Option<&Path>,
    query: Option<&str>,
) -> Result<serde_json::Value> {
    let root = Path::new(&base.output_path);
    let _lock = lock_root(root)?;
    capacity(root, policy)?;
    let started = Instant::now();
    let ledger = Ledger::open(&root.join("ledger.redb"))?;
    if let Some(payload) = Index::open(root.join("working"))?
        .inverted_index
        .commit_payload()?
    {
        ledger.apply(&payload)?;
    }
    initialize_context(&ledger, base)?;
    let restored = backfill(base, policy, &ledger, folder)?;
    let id = uuid::Uuid::new_v4().to_string();
    let directory = id_path(root, &id)?;
    fs::create_dir_all(&directory)?;
    let created = chrono::Utc::now().to_rfc3339();
    atomic(
        &directory.join("progress.json"),
        &serde_json::json!({"version":1,"id":id,"state":"preparing","created_at":created,"documents":0}),
    )?;
    status(root)?;
    let result = (|| -> Result<Manifest> {
        let mut signal_hashes = signals(base)?;
        let graph_binding = policy
            .graph_generation
            .as_ref()
            .map(|path| crate::webgraph::signals::Bundle::load(path))
            .transpose()?;
        if let Some(bundle) = &graph_binding {
            signal_hashes["graph_generation"] = bundle.binding.clone();
            signal_hashes["adopted"] = serde_json::json!(true);
            signal_hashes["prepared_values"] = serde_json::json!(
                "content preserved; centrality fields replaced from one verified generation"
            );
        }
        let mut contexts = BTreeMap::new();
        let tx = ledger.db.begin_read()?;
        let current = tx.open_table(CURRENT)?;
        let prepared = tx.open_table(PREPARED)?;
        let mut digests: Vec<_> = (0..BUCKETS)
            .map(|_| ring::digest::Context::new(&ring::digest::SHA256))
            .collect();
        let signal_binding_hash =
            crate::capture::sha256(&serde_json::to_vec(&signal_hashes.get("graph_generation"))?);
        for digest in &mut digests {
            digest.update(signal_binding_hash.as_bytes());
        }
        let mut documents = 0;
        let mut catalog = std::io::BufWriter::new(File::create(directory.join("catalog.jsonl"))?);
        let mut logical = ring::digest::Context::new(&ring::digest::SHA256);
        for entry in current.iter()? {
            stop_check()?;
            let (_, value) = entry?;
            let row: Decision = serde_json::from_slice(value.value())?;
            let context = tx
                .open_table(PREPARED_CONTEXTS)?
                .get(row.version.as_str())?
                .map(|v| serde_json::from_slice::<serde_json::Value>(v.value()))
                .transpose()?;
            if let Some(context) = &context {
                contexts.insert(
                    context["id"]
                        .as_str()
                        .context("invalid preparation context")?
                        .to_string(),
                    context["context"].clone(),
                );
                anyhow::ensure!(
                    contexts.len() <= 4096,
                    "preparation context catalog exceeds quota"
                );
            }
            let encoded = serde_json::to_vec(
                &serde_json::json!({"generation":id,"decision":row,"preparation_context":context.as_ref().map(|v|&v["id"])}),
            )?;
            catalog.write_all(&encoded)?;
            catalog.write_all(b"\n")?;
            let stable = serde_json::to_vec(
                &serde_json::json!({"url":row.url,"version":row.version,"outcome":row.outcome,"document":row.indexed_document_sha256}),
            )?;
            logical.update(&(stable.len() as u64).to_le_bytes());
            logical.update(&stable);
            if row.outcome == "indexed" {
                let doc = prepared
                    .get(row.version.as_str())?
                    .context("generation partial: prepared document absent")?;
                let digest = crate::capture::sha256(doc.value());
                let b = bucket(&row.url);
                digests[b].update(row.url.as_bytes());
                digests[b].update(row.version.as_bytes());
                digests[b].update(digest.as_bytes());
                documents += 1;
            }
        }
        catalog.flush()?;
        drop(catalog);
        let bucket_hashes: Vec<_> = digests
            .into_iter()
            .map(|v| crate::capture::hex(v.finish().as_ref()))
            .collect();
        let previous = selector(root)?;
        let prior = previous
            .as_ref()
            .map(|v| validate_manifest(root, &v.active, true))
            .transpose()?;
        let mut bucket_segments = vec![Vec::new(); BUCKETS];
        let changed: Vec<_> = (0..BUCKETS)
            .filter(|b| {
                prior
                    .as_ref()
                    .is_none_or(|p| p.bucket_hashes[*b] != bucket_hashes[*b])
            })
            .collect();
        if let Some(prior) = &prior {
            clone_index(&id_path(root, &prior.id)?, &directory)?;
            bucket_segments = prior.bucket_segments.clone();
            let remove = changed
                .iter()
                .flat_map(|b| bucket_segments[*b].iter())
                .map(|id| tantivy::index::SegmentId::from_uuid_string(id))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut index = Index::open(&directory)?;
            index.inverted_index.delete_segments_by_id(&remove)?;
            drop(index);
        }
        for &b in &changed {
            stop_check()?;
            capacity(root, policy)?;
            let before = if directory.join("inverted_index/meta.json").exists() {
                segments(&directory)?
            } else {
                vec![]
            };
            let mut index = Index::open(&directory)?;
            index
                .inverted_index
                .prepare_writer_with_budget(policy.writer_bytes)?;
            let mut queued = 0;
            for entry in current.iter()? {
                let (_, value) = entry?;
                let row: Decision = serde_json::from_slice(value.value())?;
                if row.outcome != "indexed" || bucket(&row.url) != b {
                    continue;
                }
                stop_check()?;
                let bytes = prepared
                    .get(row.version.as_str())?
                    .context("prepared document lost")?;
                let doc = decode_prepared(
                    bytes.value(),
                    graph_binding
                        .as_ref()
                        .map(|bundle| (bundle, row.url.as_str())),
                )?;
                let cost = doc.node_data.capacity() + 4096;
                anyhow::ensure!(
                    cost <= policy.prepared_bytes,
                    "generation document exceeds byte quota"
                );
                if queued + cost > policy.prepared_bytes {
                    index.commit()?;
                    queued = 0;
                }
                index.inverted_index.add_prepared_document(doc)?;
                queued += cost;
            }
            index.commit()?;
            index.inverted_index.stop();
            bucket_segments[b] = segments(&directory)?
                .into_iter()
                .filter(|id| !before.contains(id))
                .collect();
        }
        if !directory.join("inverted_index/meta.json").exists() {
            let mut index = Index::open(&directory)?;
            index.commit()?;
            index.inverted_index.stop();
        }
        let index = Index::open(&directory)?;
        anyhow::ensure!(
            index.inverted_index.num_documents() == documents && documents == ledger.documents()?,
            "generation content/count reconciliation failed"
        );
        let seen = Database::builder()
            .set_cache_size(1024 * 1024)
            .create(directory.join("validation-seen.redb"))?;
        let seen_table: TableDefinition<&str, ()> = TableDefinition::new("urls");
        let w = seen.begin_write()?;
        {
            w.open_table(seen_table)?;
        }
        w.commit()?;
        let mut checked = 0;
        for candidate in index.inverted_index.stored_content()? {
            stop_check()?;
            let (url, title, body) = candidate?;
            let row = ledger
                .current(&url)?
                .context("generation contains unknown URL")?;
            anyhow::ensure!(
                row.outcome == "indexed"
                    && row
                        .indexed_document_sha256
                        .as_ref()
                        .is_some_and(|h| *h == super::document_identity(&url, &title, &body)),
                "generation has stale/noindex/changed content"
            );
            let mut w = seen.begin_write()?;
            w.set_durability(redb::Durability::None);
            {
                let mut t = w.open_table(seen_table)?;
                anyhow::ensure!(
                    t.get(url.as_str())?.is_none(),
                    "duplicate logical generation URL"
                );
                t.insert(url.as_str(), ())?;
            }
            w.commit()?;
            checked += 1;
        }
        anyhow::ensure!(checked == documents, "generation coverage mismatch");
        drop(seen);
        fs::remove_file(directory.join("validation-seen.redb"))?;
        let smoke = if let Some(query) = query {
            let hits = index
                .inverted_index
                .preview_documents(query)?
                .collect::<Result<Vec<_>>>()?;
            anyhow::ensure!(!hits.is_empty(), "known smoke query has no results");
            serde_json::json!({"query":query,"hits":hits.len()})
        } else {
            serde_json::Value::Null
        };
        drop(index);
        let serving_smoke = if let Some(query) = query {
            let value = serving_query(&directory, query)?;
            anyhow::ensure!(
                !value["results"]
                    .as_array()
                    .context("missing serving results")?
                    .is_empty(),
                "serving reader smoke has no results"
            );
            value
        } else {
            serde_json::Value::Null
        };
        let mut decisions =
            std::io::BufWriter::new(File::create(directory.join("decisions.jsonl"))?);
        for item in tx.open_table(DECISIONS)?.iter()? {
            let (_, v) = item?;
            decisions.write_all(v.value())?;
            decisions.write_all(b"\n")?;
        }
        decisions.flush()?;
        drop(decisions);
        let mut inputs = std::io::BufWriter::new(File::create(directory.join("inputs.jsonl"))?);
        for item in tx.open_table(INPUTS)?.iter()? {
            let (k, v) = item?;
            serde_json::to_writer(
                &mut inputs,
                &serde_json::json!({"id":k.value(),"input":serde_json::from_slice::<serde_json::Value>(v.value())?}),
            )?;
            inputs.write_all(b"\n")?;
        }
        inputs.flush()?;
        drop(inputs);
        fs::remove_file(directory.join("progress.json"))?;
        fail("generation_before_manifest");
        signal_hashes["preparation_contexts"] = serde_json::to_value(contexts)?;
        let manifest = Manifest {
            version: VERSION,
            id: id.clone(),
            state: "ready".into(),
            created_at: created.clone(),
            schema_sha256: schema_hash()?,
            extractor: "html-v1/derived-v1/prepared-v1".into(),
            engine: "tantivy-fork-0.23/schema-v1".into(),
            build: build_info(),
            config: serde_json::json!({"indexer":redact(serde_json::to_value(base)?),"indexer_sha256":crate::capture::sha256(&serde_json::to_vec(base)?),"ingestion":policy}),
            signals: signal_hashes,
            documents,
            outcomes: ledger.counts()?,
            pending: ledger.counts()?.get("pending").copied().unwrap_or(0),
            logical_sha256: crate::capture::hex(logical.finish().as_ref()),
            files: inventory(&directory)?,
            bucket_hashes,
            bucket_segments,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.,
            reused_buckets: BUCKETS - changed.len(),
            rebuilt_buckets: changed.len(),
            validation: serde_json::json!({"passed":true,"checked_documents":checked,"exact_content":true,"noindex_and_withdrawals":"excluded by current ledger","pending_input_scope":"generation of committed current ledger; unresolved captures remain pending","smoke":smoke,"serving_smoke":serving_smoke,"historical_replayed":restored}),
        };
        anyhow::ensure!(
            serde_json::to_vec(&manifest)?.len() <= 1024 * 1024,
            "generation manifest exceeds metadata budget; active selector retained"
        );
        sync_tree(&directory)?;
        atomic(&directory.join("manifest.json"), &manifest)?;
        File::open(root.join("generations"))?.sync_all()?;
        fail("generation_after_manifest");
        Ok(manifest)
    })();
    if let Err(error) = &result {
        atomic(
            &directory.join("progress.json"),
            &serde_json::json!({"version":1,"id":id,"state":"failed","created_at":created,"documents":0,"error":error.to_string()}),
        )?;
    }
    status(root)?;
    Ok(serde_json::to_value(result?)?)
}

fn acknowledgement(root: &Path, selected: &Selector) -> Result<()> {
    let manifest = validate_manifest(root, &selected.active, true)?;
    let receipt = serde_json::json!({"version":1,"state":"local_generation_selected","generation":selected.active,"sequence":selected.sequence,"manifest_sha256":selected.manifest_sha256,"catalog_sha256":manifest.files.get("catalog.jsonl"),"inputs_sha256":manifest.files.get("inputs.jsonl"),"documents":manifest.documents,"gc_allowed":false,"public_serving_ack":false});
    fs::create_dir_all(root.join("retention-acks"))?;
    atomic(
        &root
            .join("retention-acks")
            .join(format!("{}.json", selected.active)),
        &receipt,
    )?;
    let audit = root.join("publication-audit");
    fs::create_dir_all(&audit)?;
    let audit_path = audit.join(format!("{:020}.json", selected.sequence));
    if audit_path.exists() {
        let previous: serde_json::Value = json(&audit_path, 65536)?;
        anyhow::ensure!(
            previous["selector"]["manifest_sha256"] == selected.manifest_sha256,
            "publication audit sequence conflict"
        );
    } else {
        let journal = root.join("publication-journal.json");
        let action = if journal.exists() {
            let v: serde_json::Value = json(&journal, 65536)?;
            if v["selector"]["sequence"] == selected.sequence {
                v["action"].as_str().unwrap_or("repair").to_owned()
            } else {
                "repair".into()
            }
        } else {
            "repair".into()
        };
        atomic(
            &audit_path,
            &serde_json::json!({"version":1,"action":action,"at":chrono::Utc::now(),"selector":selected,"receipt":receipt}),
        )?;
    }
    atomic(
        &root.join("publication-journal.json"),
        &serde_json::json!({"version":1,"state":"completed","selector":selected,"receipt":receipt}),
    )?;
    Ok(())
}
pub fn repair(base: &IndexerConfig) -> Result<serde_json::Value> {
    let root = Path::new(&base.output_path);
    let _lock = lock_root(root)?;
    let journal = root.join("publication-journal.json");
    if journal.exists() {
        let value: serde_json::Value = json(&journal, 65536)?;
        anyhow::ensure!(
            value["version"] == 1,
            "unsupported publication journal version"
        );
    }
    if let Some(selected) = selector(root)? {
        serving_path(root)?;
        acknowledgement(root, &selected)?;
    }
    status(root)
}
pub fn promote(base: &IndexerConfig, id: &str, rollback: bool) -> Result<serde_json::Value> {
    let root = Path::new(&base.output_path);
    let _lock = lock_root(root)?;
    let old = selector(root)?;
    let target = if rollback {
        old.as_ref()
            .and_then(|s| s.previous.clone())
            .context("no retained rollback generation")?
    } else {
        id.to_string()
    };
    validate_manifest(root, &target, true)?;
    if old.as_ref().is_some_and(|s| s.active == target) {
        acknowledgement(root, old.as_ref().unwrap())?;
        return status(root);
    }
    let selected = Selector {
        version: VERSION,
        sequence: old.as_ref().map_or(1, |s| s.sequence + 1),
        active: target.clone(),
        previous: old.map(|s| s.active),
        manifest_sha256: checksum(&id_path(root, &target)?.join("manifest.json"))?,
    };
    atomic(
        &root.join("publication-journal.json"),
        &serde_json::json!({"version":1,"state":"prepared","selector":selected,"action":if rollback{"rollback"}else{"promote"}}),
    )?;
    fail("promotion_before_selector");
    stop_check()?;
    atomic(&root.join("generation-selector.json"), &selected)?;
    fail("promotion_after_selector");
    acknowledgement(root, &selected)?;
    fail("promotion_after_ack");
    let error_path = root.join("publication-error.json");
    if error_path.exists() {
        fs::remove_file(error_path)?;
    }
    status(root)
}
pub fn status(root: &Path) -> Result<serde_json::Value> {
    fs::create_dir_all(root.join("generations"))?;
    let selected = selector(root)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(root.join("generations"))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if id_path(root, &id).is_err() {
            continue;
        }
        let directory = entry.path();
        let value = if directory.join("manifest.json").exists() {
            let m: Manifest = json(&directory.join("manifest.json"), 1024 * 1024)?;
            serde_json::json!({"id":m.id,"state":m.state,"documents":m.documents,"created_at":m.created_at,"logical_sha256":m.logical_sha256,"elapsed_ms":m.elapsed_ms,"reused_buckets":m.reused_buckets,"rebuilt_buckets":m.rebuilt_buckets})
        } else if directory.join("progress.json").exists() {
            json::<serde_json::Value>(&directory.join("progress.json"), 65536)?
        } else {
            serde_json::json!({"id":id,"state":"abandoned","documents":0,"created_at":"1970-01-01T00:00:00Z"})
        };
        let mut value = value;
        for (key, default) in [
            ("logical_sha256", serde_json::json!("")),
            ("elapsed_ms", serde_json::json!(0)),
            ("reused_buckets", serde_json::json!(0)),
            ("rebuilt_buckets", serde_json::json!(0)),
        ] {
            if value.get(key).is_none() {
                value[key] = default;
            }
        }
        entries.push(value);
    }
    entries.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
    entries.truncate(32);
    // Keep active/previous visible even after a longer history of local preparations.
    for id in selected
        .iter()
        .flat_map(|s| std::iter::once(&s.active).chain(s.previous.iter()))
    {
        if !entries.iter().any(|e| e["id"].as_str() == Some(id)) {
            let m = validate_manifest(root, id, false)?;
            entries.pop();
            entries.push(serde_json::json!({"id":m.id,"state":"ready","documents":m.documents,"created_at":m.created_at,"logical_sha256":m.logical_sha256,"elapsed_ms":m.elapsed_ms,"reused_buckets":m.reused_buckets,"rebuilt_buckets":m.rebuilt_buckets}));
        }
    }
    let adoption = selected
        .as_ref()
        .map(|s| {
            validate_manifest(root, &s.active, false)
                .map(|m| m.signals.get("graph_generation").cloned())
        })
        .transpose()?
        .flatten();
    let value = serde_json::json!({"version":1,"updated_at":chrono::Utc::now(),"active":selected.as_ref().map(|s|&s.active),"previous":selected.as_ref().and_then(|s|s.previous.as_ref()),"generations":entries,"signals_adoption":adoption,"artifact":build_info(),"gc_enabled":false,"public_serving_rollout":false,"last_error":if root.join("publication-error.json").exists(){Some(json::<serde_json::Value>(&root.join("publication-error.json"),65536)?)}else{None}});
    atomic(&root.join("generations/status.json"), &value)?;
    Ok(value)
}
fn serving_query(directory: &Path, query: &str) -> Result<serde_json::Value> {
    use crate::ranking::pipeline::RankableWebpage;
    let mut index = Index::open(directory)?;
    index.set_shard_id(crate::inverted_index::ShardId::Backbone(0));
    let documents = index.inverted_index.num_documents();
    let searcher = crate::searcher::LocalSearcher::builder(std::sync::Arc::new(
        tokio::sync::RwLock::new(index),
    ))
    .build();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let query = crate::searcher::SearchQuery {
        query: query.to_owned(),
        num_results: 10,
        safe_search: false,
        ..Default::default()
    };
    let (pages, signals) = rt.block_on(async {
        let initial = searcher.search_initial(&query, false).await?;
        let pointers: Vec<_> = initial
            .websites
            .iter()
            .map(|page| page.pointer().clone())
            .collect();
        let signals:Vec<_>=initial.websites.iter().map(|page|serde_json::json!({"host_centrality":page.signals().get(crate::ranking::SignalEnum::from(crate::ranking::signals::HostCentrality)).map(|v|v.value),"page_centrality":page.signals().get(crate::ranking::SignalEnum::from(crate::ranking::signals::PageCentrality)).map(|v|v.value),"score":page.score()})).collect();
        Ok::<_,anyhow::Error>((searcher.retrieve_websites(&pointers, &query.query).await?,signals))
    })?;
    let results: Vec<_> = pages.into_iter().map(|page| serde_json::json!({"url":page.url,"title":page.title.chars().take(512).collect::<String>(),"snippet":if page.body.is_empty(){page.dirty_body.chars().take(240).collect::<String>()}else{page.body.chars().take(240).collect::<String>()},"snippet_source":if page.body.is_empty(){"all_text"}else{"clean_text"},"engine_snippet":page.snippet,"region":page.region})).collect();
    Ok(
        serde_json::json!({"version":1,"documents":documents,"results":results,"reader":"LocalSearcher","ranking_signals":signals}),
    )
}
pub fn query(base: &IndexerConfig, query: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(
        query.len() <= 4096 && query.split_whitespace().count() <= 16,
        "query exceeds budget"
    );
    let root = Path::new(&base.output_path);
    if let Some(selected) = selector(root)? {
        let manifest = validate_manifest(root, &selected.active, false)?;
        anyhow::ensure!(
            checksum(&id_path(root, &selected.active)?.join("manifest.json"))?
                == selected.manifest_sha256,
            "selected manifest checksum mismatch"
        );
        let mut value = serving_query(&id_path(root, &selected.active)?, query)?;
        value["generation"] = selected.active.into();
        value["documents"] = manifest.documents.into();
        value["serving_scope"] = "local validated generation".into();
        Ok(value)
    } else {
        serving_query(&serving_path(root)?, query)
    }
}

pub fn record_publication_error(base: &IndexerConfig, error: &str) -> Result<()> {
    let root = Path::new(&base.output_path);
    atomic(
        &root.join("publication-error.json"),
        &serde_json::json!({"version":1,"error":error.chars().take(2000).collect::<String>(),"updated_at":chrono::Utc::now(),"retry":"next successful ingestion or explicit preparation","ingestion_committed":true}),
    )?;
    status(root)?;
    Ok(())
}
