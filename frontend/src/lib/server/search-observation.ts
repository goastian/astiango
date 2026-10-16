import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
import { env } from '$env/dynamic/private';

type QuerySample = {
  id: string; at: string; query: string; keywords: string[]; intent: string;
  intentMethod: string; page: number; outcome: string; status: number; traffic: string;
  generation?: string; engineRequestId?: string; apiMs: number; nativeCpuMs?: number; allocatedBytes?: number;
  responseBytes?: number; results?: number; cache?: string; stagesMs?: Record<string, number>;
  domReadyMs?: number; lcpMs?: number; cls?: number;
};
type Observation = { rows: QuerySample[]; timer?: ReturnType<typeof setInterval>; flushing: boolean; error: string | null; ready: Promise<void>; started?: string; run?: string };
const key = Symbol.for('astiango.frontend.observation.v1');
const shared = globalThis as typeof globalThis & { [key]?: Observation };
const path = resolve(env.FRONTEND_OBSERVATION_PATH || '../data/frontend/status.json');
const started = new Date().toISOString();
const run = randomUUID();
const state = shared[key] ??= { rows: [], flushing: false, error: null, ready: Promise.resolve() };
const captureQueries = env.FRONTEND_CAPTURE_QUERIES === undefined ? import.meta.env.DEV : env.FRONTEND_CAPTURE_QUERIES === 'true';
state.started ??= started; state.run ??= run;
const retentionMs = 24 * 3600 * 1000;
function prune() { state.rows = state.rows.filter(row => Date.now() - Date.parse(row.at) < retentionMs).slice(-256); if(!captureQueries)state.rows=state.rows.map(row=>({...row,query:'',keywords:[],intent:'unclassified'})); }
function percentile(values: number[], fraction: number) {
  const sorted = values.filter(Number.isFinite).sort((a,b)=>a-b);
  return sorted.length ? sorted[Math.ceil(sorted.length*fraction)-1] : null;
}
export function intentFor(query: string) {
  // Transparent rule, not a model or a fact about a person's motivation.
  if (/^https?:\/\/|^[\w.-]+\.[a-z]{2,}(?:\/|$)/i.test(query)) return 'navigational';
  if (/\b(buy|price|comprar|precio|kaufen|acheter|preço|prezzo)\b|购买|价格/i.test(query)) return 'commercial';
  if (/\b(how|what|why|cómo|qué|por qué|wie|was|comment|pourquoi|como|come|cosa)\b|如何|什么/i.test(query)) return 'informational';
  return 'unclassified';
}
export async function recordSearch(row: Omit<QuerySample, 'at' | 'keywords' | 'intent' | 'intentMethod'>) {
  await state.ready;
  state.rows.push({ ...row, query: captureQueries ? row.query.slice(0,512) : '', at: new Date().toISOString(),
    keywords: captureQueries ? [...new Set(row.query.toLocaleLowerCase().match(/[\p{L}\p{N}]+/gu) ?? [])].slice(0,16) : [],
    intent: captureQueries ? intentFor(row.query) : 'unclassified', intentMethod: 'lexical-rule-v1; inferred, not verified' });
  prune();
}
export function recordExperience(id: string, value: Record<string, unknown>) {
  const row = state.rows.find(row => row.id === id);
  if (!row) return;
  for (const name of ['domReadyMs','lcpMs','cls'] as const) {
    const n = value[name];
    if (typeof n === 'number' && Number.isFinite(n) && n >= 0 && n <= (name === 'cls' ? 10 : 60000)) row[name] = n;
  }
}
async function flush() {
  if (state.flushing) return;
  state.flushing = true;
  try {
    prune();
    const users = state.rows.filter(row=>row.traffic==='user');
    const window = users.filter(row=>Date.now()-Date.parse(row.at)<60000);
    const sum = (name: 'nativeCpuMs' | 'responseBytes') => users.reduce((total,row)=>total+(row[name]??0),0);
    const rates = { cpuHour: Number(env.SEARCH_CPU_HOUR_PRICE), egressGiB: Number(env.SEARCH_EGRESS_GIB_PRICE) };
    const priced = !!env.SEARCH_CPU_HOUR_PRICE?.trim() && !!env.SEARCH_EGRESS_GIB_PRICE?.trim() && Object.values(rates).every(v=>Number.isFinite(v)&&v>=0);
    const value = { schema_version: 1, component: 'frontend', pid: process.pid, started_at: state.started,
      run_id: state.run, updated_at: new Date().toISOString(), telemetry_error: state.error,
      retention: { max_queries: 256, max_hours: 24, raw_queries: captureQueries, scope: 'local operator; user requested query diagnostics; no IP or cookies' },
      build: { url: env.FRONTEND_PUBLIC_URL || 'http://127.0.0.1:8002/', mode: import.meta.env.DEV ? 'vite-hmr' : 'production', api: 'same-origin adapter → local Search/API v1' },
      resources: { rss_bytes: process.memoryUsage().rss, heap_bytes: process.memoryUsage().heapUsed },
      window: { samples: window.length, qps: window.length/60,
        p50_ms: percentile(window.map(r=>r.apiMs),.5), p95_ms: percentile(window.map(r=>r.apiMs),.95), p99_ms: percentile(window.map(r=>r.apiMs),.99) },
      cost: { samples: users.length, cpu_samples: users.filter(row=>Number.isFinite(row.nativeCpuMs)).length, byte_samples: users.filter(row=>Number.isFinite(row.responseBytes)).length, native_cpu_ms: sum('nativeCpuMs'), response_bytes: sum('responseBytes'),
        price_configured: priced, rates: priced ? rates : null, currency: env.SEARCH_PRICE_CURRENCY || 'USD',
        estimated_variable_cost: priced ? sum('nativeCpuMs')/3600000*rates.cpuHour + sum('responseBytes')/1073741824*rates.egressGiB : null,
        scope: 'native query CPU + API JSON bytes only; excludes idle readers, RAM, storage, ingestion, frontend CPU and browser' },
      queries: state.rows, history: users.slice(-120).map(r=>({at:r.at,latency_ms:r.apiMs,cpu_ms:r.nativeCpuMs??0,bytes:r.responseBytes??0})) };
    await mkdir(dirname(path), { recursive: true });
    const temporary = `${path}.${process.pid}.tmp`;
    await writeFile(temporary, JSON.stringify(value));
    await rename(temporary,path);
    state.error = null;
  } catch (error) { state.error = error instanceof Error ? error.message : String(error); }
  finally { state.flushing = false; }
}
if (!state.timer) {
  state.ready = readFile(path,'utf8').then(text=>{
    const value=JSON.parse(text);
    if(value.schema_version===1 && Array.isArray(value.queries)) state.rows=value.queries.slice(-256);
    prune();
  }).catch(()=>{});
}
// Replace the projector on HMR while retaining the ring and counters.
if(state.timer)clearInterval(state.timer);
state.timer=setInterval(()=>void state.ready.then(flush),1000);
state.timer.unref();
