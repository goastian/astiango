//! Versioned harmonic signals. Exact identity is separate from the legacy reader key.
use super::{
    metrics,
    pipeline::{Config, Document, StoredEdge},
    Node, NodeID,
};
use crate::{
    capture, crawler::identity::UrlIdentity, hyperloglog::HyperLogLog, warc::consumer,
    webpage::html::links::RelFlags,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use url::Url;

#[derive(Clone, Serialize, Deserialize)]
pub struct Value {
    pub node: String,
    pub score: f64,
    pub rank: u64,
    pub legacy_ids: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Summary {
    pub version: u32,
    pub algorithm: String,
    pub scope: String,
    pub direction: String,
    pub normalization: String,
    pub excluded_flags: u64,
    pub seed: u64,
    pub config_sha256: String,
    pub nodes: usize,
    pub edges: usize,
    pub filtered_edges: usize,
    pub iterations: usize,
    pub cpu_ms: f64,
    pub elapsed_ms: f64,
    pub reserved_bytes: usize,
    pub absolute_error_bound: f64,
    pub legacy_collisions: usize,
    pub top: Vec<Value>,
}
pub fn contract(c: &Config) -> serde_json::Value {
    serde_json::json!({"version":2,"algorithm":"exact-frontier-bitsets-v1","scope":c.centrality_scope,
        "direction":"incoming","normalization":"sum(1/d)/(N-1); self excluded; singleton zero",
        "excluded_flags":(RelFlags::NOFOLLOW|RelFlags::SPONSORED|RelFlags::UGC).as_u64(),
        "same_icann_domain_is_not_excluded":true,"seed":0,"max_nodes":c.centrality_max_nodes,
        "max_edges":c.centrality_max_edges,"memory_bytes":c.centrality_memory_bytes,
        "max_iterations":c.centrality_max_iterations,"timeout_ms":c.centrality_timeout_ms,
        "absolute_error_bound":1e-12,"score_rounding":"nearest 1e-14; deterministic ties by exact identity","legacy_projection":"maximum score across mapped identities"})
}
pub fn contract_hash(c: &Config) -> Result<String> {
    Ok(capture::sha256(&serde_json::to_vec(&contract(c))?))
}
pub fn key(url: &Url, scope: &str) -> Result<String> {
    let identity = UrlIdentity::from_url(url)?;
    Ok(match scope {
        "host" => identity.host,
        "tenant" => identity.tenant,
        "page" => identity.url,
        _ => anyhow::bail!("unknown centrality identity scope: {scope}"),
    })
}
fn lines<T: serde::de::DeserializeOwned>(
    path: &Path,
    mut visit: impl FnMut(T) -> Result<()>,
) -> Result<()> {
    let mut reader = BufReader::with_capacity(65536, File::open(path)?);
    loop {
        let mut bytes = Vec::new();
        reader
            .by_ref()
            .take(2 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut bytes)?;
        if bytes.is_empty() {
            break;
        }
        anyhow::ensure!(
            bytes.len() <= 2 * 1024 * 1024 && bytes.last() == Some(&b'\n'),
            "signal record exceeds line quota or is truncated"
        );
        visit(serde_json::from_slice(&bytes)?)?;
    }
    Ok(())
}
struct Graph {
    names: Vec<String>,
    aliases: Vec<BTreeSet<String>>,
    edges: Vec<(usize, usize)>,
    filtered: usize,
    bytes: usize,
}
impl Graph {
    fn load(root: &Path, c: &Config) -> Result<Self> {
        let mut map = BTreeMap::<String, usize>::new();
        let mut names = Vec::new();
        let mut aliases = Vec::<BTreeSet<String>>::new();
        let mut bytes = 0usize;
        let mut node = |raw: &str| -> Result<usize> {
            let url = Url::parse(raw)?;
            let name = key(&url, &c.centrality_scope)?;
            let legacy = if c.centrality_scope == "page" {
                Node::from(url).id()
            } else {
                Node::from(url).into_host().id()
            };
            let index = if let Some(i) = map.get(&name) {
                *i
            } else {
                anyhow::ensure!(
                    names.len() < c.centrality_max_nodes,
                    "centrality node quota; previous scores retained"
                );
                bytes = bytes.saturating_add(name.len() * 2 + 256);
                anyhow::ensure!(
                    bytes < c.centrality_memory_bytes / 2,
                    "centrality identity byte quota"
                );
                let i = names.len();
                map.insert(name.clone(), i);
                names.push(name);
                aliases.push(BTreeSet::new());
                i
            };
            let alias = format!("{:032x}", legacy.as_u128());
            if aliases[index].insert(alias) {
                bytes += 96;
            }
            Ok(index)
        };
        lines::<Document>(&root.join("catalog.jsonl"), |doc| {
            if doc.state != "withdrawn" {
                node(&doc.final_url)?;
            }
            Ok(())
        })?;
        #[derive(Deserialize)]
        struct Row {
            edge: StoredEdge,
        }
        let mut edges = Vec::new();
        let mut filtered = 0;
        lines::<Row>(&root.join("edges.jsonl"), |row| {
            if row
                .edge
                .rel
                .intersects(RelFlags::NOFOLLOW | RelFlags::SPONSORED | RelFlags::UGC)
            {
                filtered += 1;
                return Ok(());
            }
            let from = node(&row.edge.source_url)?;
            let to = node(&row.edge.destination_url)?;
            if from != to {
                anyhow::ensure!(
                    edges.len() < c.centrality_max_edges,
                    "centrality edge quota; previous scores retained"
                );
                edges.push((from, to));
            }
            Ok(())
        })?;
        drop(node);
        edges.sort_unstable();
        edges.dedup();
        let mut order: Vec<_> = (0..names.len()).collect();
        order.sort_by(|a, b| names[*a].cmp(&names[*b]));
        let mut remap = vec![0; names.len()];
        for (i, old) in order.iter().enumerate() {
            remap[*old] = i;
        }
        for (from, to) in &mut edges {
            *from = remap[*from];
            *to = remap[*to];
        }
        edges.sort_unstable();
        let sorted_names = order.iter().map(|i| names[*i].clone()).collect();
        let sorted_aliases = order.iter().map(|i| aliases[*i].clone()).collect();
        bytes += edges.capacity() * std::mem::size_of::<(usize, usize)>();
        Ok(Self {
            names: sorted_names,
            aliases: sorted_aliases,
            edges,
            filtered,
            bytes,
        })
    }
    fn outgoing(&self) -> Vec<Vec<usize>> {
        let mut result = vec![Vec::new(); self.names.len()];
        for &(a, b) in &self.edges {
            result[a].push(b);
        }
        result
    }
}
fn check(c: &Config, start: Instant) -> Result<()> {
    anyhow::ensure!(
        !super::pipeline::CANCELLED.load(std::sync::atomic::Ordering::Relaxed),
        "cancelled: centrality drained; previous scores retained"
    );
    anyhow::ensure!(
        start.elapsed().as_millis() < c.centrality_timeout_ms as u128,
        "centrality timeout; previous scores retained"
    );
    Ok(())
}
fn exact(graph: &Graph, c: &Config) -> Result<(Vec<f64>, usize, usize)> {
    let n = graph.names.len();
    let words = n.div_ceil(64);
    let reserve = n
        .checked_mul(words)
        .and_then(|x| x.checked_mul(24))
        .context("centrality matrix overflow")?
        .saturating_add(graph.bytes)
        .saturating_add(graph.edges.len() * 16 + n * 128);
    anyhow::ensure!(
        reserve <= c.centrality_memory_bytes,
        "centrality byte quota; previous scores retained"
    );
    let mut reached = vec![0u64; n * words];
    let mut frontier = reached.clone();
    let mut next = reached.clone();
    let mut scores = vec![0.; n];
    let adjacency = graph.outgoing();
    let mut changed = vec![true; n];
    for i in 0..n {
        reached[i * words + i / 64] |= 1 << (i % 64);
        frontier[i * words + i / 64] |= 1 << (i % 64);
    }
    if n < 2 {
        return Ok((scores, 0, reserve));
    }
    let start = Instant::now();
    for distance in 1..=c.centrality_max_iterations {
        check(c, start)?;
        next.fill(0);
        for from in 0..n {
            if from % 64 == 0 {
                check(c, start)?;
            }
            if !changed[from] {
                continue;
            }
            for &to in &adjacency[from] {
                for w in 0..words {
                    let bits = frontier[from * words + w];
                    if bits != 0 {
                        next[to * words + w] |= bits;
                    }
                }
            }
        }
        let mut any = false;
        for i in 0..n {
            let mut count = 0u64;
            for w in 0..words {
                let index = i * words + w;
                let fresh = next[index] & !reached[index];
                next[index] = fresh;
                reached[index] |= fresh;
                count += fresh.count_ones() as u64;
            }
            changed[i] = count > 0;
            any |= changed[i];
            scores[i] += count as f64 / distance as f64 / (n - 1) as f64;
        }
        if !any {
            for value in &mut scores {
                *value = (*value * 1e14).round() / 1e14;
            }
            return Ok((scores, distance, reserve));
        }
        std::mem::swap(&mut frontier, &mut next);
    }
    anyhow::bail!("centrality iteration quota before convergence; previous scores retained")
}
fn reference(graph: &Graph) -> Vec<f64> {
    let n = graph.names.len();
    let adjacency = graph.outgoing();
    let mut score = vec![0.; n];
    for source in 0..n {
        let mut distance = vec![usize::MAX; n];
        distance[source] = 0;
        let mut queue = VecDeque::from([source]);
        while let Some(from) = queue.pop_front() {
            for &to in &adjacency[from] {
                if distance[to] == usize::MAX {
                    distance[to] = distance[from] + 1;
                    queue.push_back(to);
                }
            }
        }
        for i in 0..n {
            if i != source && distance[i] != usize::MAX {
                score[i] += 1.0 / distance[i] as f64 / (n.saturating_sub(1).max(1)) as f64;
            }
        }
    }
    for value in &mut score {
        *value = (*value * 1e14).round() / 1e14;
    }
    score
}
fn approximate<const M: usize>(graph: &Graph) -> Vec<f64> {
    let n = graph.names.len();
    let mut old = vec![HyperLogLog::<M>::default(); n];
    let mut score = vec![0.; n];
    for (i, name) in graph.names.iter().enumerate() {
        let hash = capture::sha256(name.as_bytes());
        old[i].add_u128(u128::from_str_radix(&hash[..32], 16).unwrap());
    }
    for distance in 1..=n.max(1) {
        let mut next = old.clone();
        for &(from, to) in &graph.edges {
            next[to].merge(&old[from]);
        }
        let mut changed = false;
        for i in 0..n {
            changed |= next[i].registers() != old[i].registers();
            score[i] += next[i].size().saturating_sub(old[i].size()) as f64
                / distance as f64
                / n.saturating_sub(1).max(1) as f64;
        }
        old = next;
        if !changed {
            break;
        }
    }
    score
}
pub fn calculate(root: &Path, c: &Config) -> Result<Summary> {
    let start = Instant::now();
    let cpu = metrics::cpu_ns();
    let graph = Graph::load(root, c)?;
    let (scores, iterations, reserved_bytes) = {
        let _span = metrics::Span::new(8);
        exact(&graph, c)?
    };
    let _span = metrics::Span::new(9);
    let folder = root.join("scores");
    fs::create_dir_all(&folder)?;
    let mut ranked: Vec<_> = (0..scores.len()).collect();
    ranked.sort_by(|a, b| {
        scores[*b]
            .total_cmp(&scores[*a])
            .then_with(|| graph.names[*a].cmp(&graph.names[*b]))
    });
    let mut rows = Vec::new();
    let mut legacy = BTreeMap::<NodeID, (f64, u64)>::new();
    let mut collisions = 0;
    let mut json = std::io::BufWriter::new(File::create(folder.join("nodes.jsonl"))?);
    let mut csv = csv::Writer::from_path(folder.join("harmonic.csv"))?;
    for (rank, &i) in ranked.iter().enumerate() {
        check(c, start)?;
        anyhow::ensure!(
            scores[i].is_finite() && scores[i] >= 0.,
            "non-finite exact score"
        );
        let value = Value {
            node: graph.names[i].clone(),
            score: scores[i],
            rank: rank as u64,
            legacy_ids: graph.aliases[i].iter().cloned().collect(),
        };
        serde_json::to_writer(&mut json, &value)?;
        json.write_all(b"\n")?;
        for id in &value.legacy_ids {
            let id = NodeID::from(u128::from_str_radix(id, 16)?);
            if let Some(old) = legacy.get_mut(&id) {
                collisions += 1;
                if value.score > old.0 {
                    *old = (value.score, value.rank);
                }
            } else {
                legacy.insert(id, (value.score, value.rank));
            }
        }
        if rank < 20 {
            rows.push(value.clone());
        }
        csv.write_record([&value.node, &value.score.to_string()])?;
    }
    json.flush()?;
    csv.flush()?;
    let mut values: speedy_kv::Db<NodeID, f64> =
        speedy_kv::Db::open_or_create(folder.join("harmonic"))?;
    let mut ranks: speedy_kv::Db<NodeID, u64> =
        speedy_kv::Db::open_or_create(folder.join("harmonic_rank"))?;
    for (id, (score, rank)) in legacy {
        values.insert(id, score)?;
        ranks.insert(id, rank)?;
    }
    values.commit()?;
    ranks.commit()?;
    let summary = Summary {
        version: 2,
        algorithm: "exact-frontier-bitsets-v1".into(),
        scope: c.centrality_scope.clone(),
        direction: "incoming".into(),
        normalization: "sum(1/d)/(N-1); self excluded; singleton zero".into(),
        excluded_flags: (RelFlags::NOFOLLOW | RelFlags::SPONSORED | RelFlags::UGC).as_u64(),
        seed: 0,
        config_sha256: contract_hash(c)?,
        nodes: graph.names.len(),
        edges: graph.edges.len(),
        filtered_edges: graph.filtered,
        iterations,
        cpu_ms: metrics::cpu_ns().saturating_sub(cpu) as f64 / 1e6,
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
        reserved_bytes,
        absolute_error_bound: 1e-12,
        legacy_collisions: collisions,
        top: rows,
    };
    consumer::atomic_json(&folder.join("contract.json"), &contract(c))?;
    consumer::atomic_json(&folder.join("summary.json"), &summary)?;
    Ok(summary)
}

pub struct Bundle {
    pub path: PathBuf,
    pub binding: serde_json::Value,
    pub scope: String,
    values: BTreeMap<String, (f64, u64)>,
}
impl Bundle {
    pub fn load(path: &Path) -> Result<Self> {
        let manifest: super::pipeline::Manifest =
            serde_json::from_reader(File::open(path.join("manifest.json"))?.take(1024 * 1024))?;
        anyhow::ensure!(
            matches!(manifest.version, 1 | 2) && manifest.state == "ready",
            "unknown or partial graph signal generation"
        );
        let summary = manifest
            .signals
            .context("graph generation has no precision-verified signals")?;
        anyhow::ensure!(
            summary.version == 2 && summary.algorithm == "exact-frontier-bitsets-v1",
            "unknown signal algorithm/version"
        );
        for required in [
            "scores/nodes.jsonl",
            "scores/contract.json",
            "scores/summary.json",
        ] {
            anyhow::ensure!(
                manifest.artifacts.contains_key(required),
                "missing versioned signal artifact: {required}"
            );
        }
        for (relative, hash) in &manifest.artifacts {
            anyhow::ensure!(
                !Path::new(relative).is_absolute()
                    && !Path::new(relative)
                        .components()
                        .any(|c| c == std::path::Component::ParentDir),
                "signal artifact escapes generation"
            );
            anyhow::ensure!(
                consumer::checksum(File::open(path.join(relative))?)? == *hash,
                "incompatible graph/scores/ranks artifact: {relative}"
            );
        }
        let mut values = BTreeMap::new();
        lines::<Value>(&path.join("scores/nodes.jsonl"), |v| {
            anyhow::ensure!(
                v.score.is_finite() && v.score >= 0. && values.len() < 16384,
                "invalid/over-budget signal values"
            );
            anyhow::ensure!(
                values.insert(v.node, (v.score, v.rank)).is_none(),
                "duplicate signal identity"
            );
            Ok(())
        })?;
        anyhow::ensure!(
            values.len() == summary.nodes,
            "signal node counts do not reconcile"
        );
        let binding = serde_json::json!({"version":2,"generation":manifest.id,"manifest_sha256":consumer::checksum(File::open(path.join("manifest.json"))?)?,"graph_sha256":manifest.logical_sha256,"scores_sha256":manifest.artifacts.get("scores/nodes.jsonl"),"contract_sha256":summary.config_sha256,"algorithm":summary.algorithm,"scope":summary.scope,"nodes":summary.nodes,"edges":summary.edges});
        Ok(Self {
            path: path.to_owned(),
            binding,
            scope: summary.scope,
            values,
        })
    }
    pub fn lookup(&self, url: &Url) -> Result<(f64, u64)> {
        Ok(self
            .values
            .get(&key(url, &self.scope)?)
            .copied()
            .unwrap_or((0., u64::MAX)))
    }
}
pub fn compare(path: &Path, c: &Config) -> Result<serde_json::Value> {
    let graph = Graph::load(path, c)?;
    anyhow::ensure!(
        graph.names.len() <= 4096,
        "exact reference cohort exceeds quota"
    );
    let start = metrics::cpu_ns();
    let expected = reference(&graph);
    let reference_cpu = (metrics::cpu_ns() - start) as f64 / 1e6;
    let start = metrics::cpu_ns();
    let (actual, iterations, bytes) = exact(&graph, c)?;
    let exact_cpu = (metrics::cpu_ns() - start) as f64 / 1e6;
    let error = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f64::max);
    let top = |values: &[f64]| {
        let mut ids: Vec<_> = (0..values.len()).collect();
        ids.sort_by(|a, b| {
            values[*b]
                .total_cmp(&values[*a])
                .then_with(|| graph.names[*a].cmp(&graph.names[*b]))
        });
        ids.into_iter().take(10).collect::<Vec<_>>()
    };
    anyhow::ensure!(
        top(&actual) == top(&expected),
        "exact top-k order differs from reference"
    );
    let mut variants = BTreeMap::new();
    for (name, values) in [
        ("hll64", approximate::<64>(&graph)),
        ("hll256", approximate::<256>(&graph)),
        ("hll1024", approximate::<1024>(&graph)),
    ] {
        let mut relative: Vec<_> = values
            .iter()
            .zip(&expected)
            .filter(|(_, b)| **b > 1e-12)
            .map(|(a, b)| (a - b).abs() / b)
            .collect();
        relative.sort_by(f64::total_cmp);
        let top = |values: &[f64]| {
            let mut ids: Vec<_> = (0..values.len()).collect();
            ids.sort_by(|a, b| {
                values[*b]
                    .total_cmp(&values[*a])
                    .then_with(|| graph.names[*a].cmp(&graph.names[*b]))
            });
            ids.into_iter().take(10).collect::<BTreeSet<_>>()
        };
        let a = top(&values);
        let b = top(&expected);
        let overlap = a.intersection(&b).count() as f64 / b.len().max(1) as f64;
        variants.insert(name,serde_json::json!({"max_absolute_error":values.iter().zip(&expected).map(|(a,b)|(a-b).abs()).fold(0.,f64::max),"relative_p95":relative.get(relative.len().saturating_sub(1)*95/100).copied().unwrap_or(0.),"top10_overlap":overlap}));
    }
    anyhow::ensure!(error <= 1e-12, "exact score equivalence failed");
    Ok(
        serde_json::json!({"nodes":graph.names.len(),"edges":graph.edges.len(),"reference_cpu_ms":reference_cpu,"optimized_cpu_ms":exact_cpu,"iterations":iterations,"reserved_bytes":bytes,"max_absolute_error":error,"top_k_overlap":1.,"approximations":variants,"contract":contract(c)}),
    )
}
