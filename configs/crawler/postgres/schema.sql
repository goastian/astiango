CREATE SCHEMA IF NOT EXISTS crawler;

CREATE TABLE IF NOT EXISTS crawler.partitions (
  id integer PRIMARY KEY CHECK (id BETWEEN 0 AND 127),
  owner text NOT NULL,
  epoch bigint NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS crawler.origins (
  origin text PRIMARY KEY,
  partition_id integer NOT NULL REFERENCES crawler.partitions,
  epoch bigint NOT NULL DEFAULT 0,
  owner text,
  token uuid,
  deadline timestamptz,
  eligible_at timestamptz NOT NULL DEFAULT '-infinity'
);
CREATE TABLE IF NOT EXISTS crawler.urls (
  id text PRIMARY KEY,
  identity_version integer NOT NULL CHECK (identity_version = 1),
  url text NOT NULL UNIQUE,
  origin text NOT NULL REFERENCES crawler.origins,
  fetch_id uuid NOT NULL DEFAULT gen_random_uuid(),
  state text NOT NULL DEFAULT 'queued' CHECK (state IN ('queued','leased','committed','terminal')),
  attempts integer NOT NULL DEFAULT 0,
  eligible_at timestamptz NOT NULL DEFAULT '-infinity',
  reason text,
  provenance jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS urls_ready ON crawler.urls(eligible_at, id) WHERE state IN ('queued','leased');
CREATE INDEX IF NOT EXISTS origins_ready ON crawler.origins(eligible_at, origin);
CREATE TABLE IF NOT EXISTS crawler.captures (
  fetch_id uuid PRIMARY KEY,
  url_id text NOT NULL REFERENCES crawler.urls,
  epoch bigint NOT NULL,
  payload_sha256 text NOT NULL,
  payload bytea NOT NULL,
  metadata jsonb NOT NULL,
  committed_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE IF NOT EXISTS crawler.outbox (
  fetch_id uuid PRIMARY KEY REFERENCES crawler.captures,
  discoveries jsonb NOT NULL,
  published_at timestamptz
);
ALTER TABLE crawler.outbox ADD COLUMN IF NOT EXISTS published_count integer NOT NULL DEFAULT 0 CHECK (published_count >= 0);
CREATE INDEX IF NOT EXISTS outbox_pending ON crawler.outbox(fetch_id) WHERE published_at IS NULL;
CREATE TABLE IF NOT EXISTS crawler.commands (
  id uuid PRIMARY KEY,
  reply jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- Online additions retain existing URL/fetch identities and pending work.
ALTER TABLE crawler.origins ADD COLUMN IF NOT EXISTS last_turn timestamptz NOT NULL DEFAULT '-infinity';
ALTER TABLE crawler.urls ADD COLUMN IF NOT EXISTS continuation jsonb;
ALTER TABLE crawler.urls ADD COLUMN IF NOT EXISTS next_origin text REFERENCES crawler.origins;
ALTER TABLE crawler.urls ADD COLUMN IF NOT EXISTS http_attempts integer NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS crawler.budgets (
  day date NOT NULL, scope text NOT NULL, requests bigint NOT NULL DEFAULT 0,
  reserved_bytes bigint NOT NULL DEFAULT 0, PRIMARY KEY(day,scope)
);
CREATE TABLE IF NOT EXISTS crawler.ip_slots (
  fetch_id uuid NOT NULL, ip inet NOT NULL, deadline timestamptz NOT NULL,
  PRIMARY KEY(fetch_id,ip)
);
CREATE INDEX IF NOT EXISTS ip_slots_active ON crawler.ip_slots(ip,deadline);
CREATE INDEX IF NOT EXISTS urls_origin_ready ON crawler.urls(origin,eligible_at,id) WHERE state IN ('queued','leased');

CREATE TABLE IF NOT EXISTS crawler.request_slots(fetch_id uuid PRIMARY KEY,deadline timestamptz NOT NULL);
CREATE INDEX IF NOT EXISTS request_slots_expiry ON crawler.request_slots(deadline);
CREATE TABLE IF NOT EXISTS crawler.auxiliary_origins (fetch_id uuid NOT NULL,origin text NOT NULL REFERENCES crawler.origins,
  token uuid NOT NULL,epoch bigint NOT NULL,PRIMARY KEY(fetch_id,origin));
CREATE TABLE IF NOT EXISTS crawler.robots_cache (origin text PRIMARY KEY REFERENCES crawler.origins,
  status integer NOT NULL, body text NOT NULL, expires_at timestamptz NOT NULL);
ALTER TABLE crawler.robots_cache ADD COLUMN IF NOT EXISTS crawl_delay_ms bigint NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS crawler.capacity (id boolean PRIMARY KEY DEFAULT true CHECK(id), urls bigint NOT NULL);
INSERT INTO crawler.capacity SELECT true,count(*) FROM crawler.urls ON CONFLICT DO NOTHING;
CREATE TABLE IF NOT EXISTS crawler.transfers (partition_id integer PRIMARY KEY REFERENCES crawler.partitions,
  target text NOT NULL, phase text NOT NULL CHECK(phase IN ('draining','complete')), started_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE OR REPLACE FUNCTION crawler.admit_url(item jsonb, maximum bigint) RETURNS boolean LANGUAGE plpgsql SET search_path=crawler,pg_catalog AS $admit$
BEGIN
  IF EXISTS(SELECT FROM urls WHERE id=item->>'id') THEN RETURN false; END IF;
  PERFORM FROM capacity WHERE id=true FOR UPDATE;
  IF EXISTS(SELECT FROM urls WHERE id=item->>'id') THEN RETURN false; END IF;
  IF (SELECT urls FROM capacity WHERE id=true)>=maximum THEN RAISE EXCEPTION 'frontier_quota'; END IF;
  INSERT INTO origins(origin,partition_id) VALUES(item->>'origin',(item->>'partition_id')::integer) ON CONFLICT DO NOTHING;
  INSERT INTO urls(id,identity_version,url,origin,provenance) VALUES(item->>'id',1,item->>'url',item->>'origin',coalesce(item->'provenance','{}'));
  UPDATE capacity SET urls=urls+1 WHERE id=true;
  RETURN true;
END $admit$;

CREATE INDEX IF NOT EXISTS origins_partition ON crawler.origins(partition_id);
CREATE INDEX IF NOT EXISTS ip_slots_expiry ON crawler.ip_slots(deadline);
CREATE INDEX IF NOT EXISTS robots_expiry ON crawler.robots_cache(expires_at);
CREATE INDEX IF NOT EXISTS commands_expiry ON crawler.commands(created_at);

CREATE OR REPLACE FUNCTION crawler.rpc(request jsonb, replicas integer)
RETURNS jsonb LANGUAGE plpgsql SET search_path = crawler,pg_catalog AS $$
DECLARE
  action text := request->>'action';
  node text := request->>'node_id';
  worker text := request->>'worker_id';
  now_at timestamptz := clock_timestamp();
  duration_ms bigint := (request->>'lease_ms')::bigint;
  cooldown_ms bigint := (request->>'cooldown_ms')::bigint;
  candidate urls%ROWTYPE;
  authority origins%ROWTYPE;
  reply jsonb;
  existing jsonb;
  item jsonb;
  stored captures%ROWTYPE;
  publication outbox%ROWTYPE;
  budget budgets%ROWTYPE;
  scope_key text;
  address text;
  quota bigint;
BEGIN
  IF coalesce((request->>'version')::integer,0) <> 2 THEN
    RETURN jsonb_build_object('version',2,'error','unsupported_version');
  END IF;
  IF pg_is_in_recovery() THEN RETURN jsonb_build_object('version',2,'error','not_primary'); END IF;
  IF replicas > 0 AND (SELECT count(*) FROM pg_stat_replication WHERE state='streaming' AND sync_state IN ('sync','quorum')) < replicas THEN
    RETURN jsonb_build_object('version',2,'error','no_quorum');
  END IF;
  PERFORM set_config('synchronous_commit', CASE WHEN replicas > 0 THEN 'remote_apply' ELSE 'on' END, true);
  PERFORM pg_advisory_xact_lock(hashtextextended(request->>'command_id',0));
  SELECT c.reply INTO existing FROM commands c WHERE id=(request->>'command_id')::uuid;
  IF existing IS NOT NULL THEN RETURN existing; END IF;
  DELETE FROM commands WHERE id IN (SELECT id FROM commands WHERE created_at<now_at-interval '7 days' ORDER BY created_at LIMIT 128 FOR UPDATE SKIP LOCKED);
  IF action='publish_outbox' THEN
    SELECT * INTO publication FROM outbox WHERE fetch_id=(request->>'fetch_id')::uuid FOR UPDATE;
    IF publication.fetch_id IS NULL THEN RETURN jsonb_build_object('version',2,'error','unknown_outbox'); END IF;
    IF publication.published_at IS NOT NULL THEN RETURN jsonb_build_object('version',2,'published',0,'duplicate',true); END IF;
    IF publication.published_count IS DISTINCT FROM (request->>'cursor')::integer THEN
      RETURN jsonb_build_object('version',2,'error','outbox_changed');
    END IF;
    FOR item IN SELECT value FROM jsonb_array_elements(request->'urls') LOOP
      PERFORM crawler.admit_url(item,(request->>'maximum_urls')::bigint);
    END LOOP;
    UPDATE outbox SET published_count=(request->>'next_cursor')::integer,
      published_at=CASE WHEN (request->>'finished')::boolean THEN now_at ELSE NULL END WHERE fetch_id=publication.fetch_id;
    reply:=jsonb_build_object('version',2,'published',jsonb_array_length(request->'urls'),'finished',(request->>'finished')::boolean);
  ELSIF action='admit' THEN
    FOR item IN SELECT value FROM jsonb_array_elements(request->'urls') LOOP
      PERFORM crawler.admit_url(item,(request->>'maximum_urls')::bigint);
    END LOOP;
    reply := jsonb_build_object('version',2,'admitted',jsonb_array_length(request->'urls'));
  ELSIF action='claim' THEN
    IF duration_ms IS NULL OR cooldown_ms IS NULL OR duration_ms NOT BETWEEN 100 AND 3600000 OR cooldown_ms NOT BETWEEN 0 AND 3600000 THEN
      RETURN jsonb_build_object('version',2,'error','invalid_lease_budget');
    END IF;
    SELECT u.* INTO candidate FROM urls u WHERE u.id=(
      SELECT first_url.id FROM origins o JOIN partitions p ON p.id=o.partition_id
      CROSS JOIN LATERAL (SELECT ready.id FROM urls ready WHERE ready.origin=o.origin
        AND ready.state IN ('queued','leased') AND ready.eligible_at<=now_at
        ORDER BY ready.eligible_at,ready.id LIMIT 1) first_url
      WHERE p.owner=node AND o.eligible_at<=now_at
        AND (o.deadline IS NULL OR o.deadline+cooldown_ms*interval '1 millisecond'<=now_at)
      ORDER BY o.last_turn,o.origin LIMIT 1)
      FOR UPDATE OF u SKIP LOCKED;
    IF candidate.id IS NULL THEN RETURN jsonb_build_object('version',2,'lease',NULL,'retry_after_ms',100); END IF;
    SELECT * INTO authority FROM origins WHERE origin=candidate.origin FOR UPDATE;
    now_at := clock_timestamp();
    IF authority.eligible_at>now_at OR authority.deadline + cooldown_ms * interval '1 millisecond'>now_at THEN
      RETURN jsonb_build_object('version',2,'lease',NULL,'retry_after_ms',100);
    END IF;
    UPDATE origins SET last_turn=now_at,epoch=epoch+1,owner=worker,token=gen_random_uuid(),deadline=now_at+duration_ms*interval '1 millisecond' WHERE origin=candidate.origin RETURNING * INTO authority;
    UPDATE urls SET state='leased' WHERE id=candidate.id;
    reply := jsonb_build_object('version',2,'lease',jsonb_build_object('url',candidate.url,'url_id',candidate.id,'origin',candidate.origin,
      'fetch_id',candidate.fetch_id,'epoch',authority.epoch,'token',authority.token,'owner',worker,'attempts',candidate.attempts+1,'continuation',candidate.continuation,'lease_ms',duration_ms,'server_now',now_at));
  ELSE
    SELECT * INTO candidate FROM urls WHERE id=request->>'url_id' FOR UPDATE;
    IF candidate.id IS NULL THEN RETURN jsonb_build_object('version',2,'error','unknown_url'); END IF;
    SELECT * INTO authority FROM origins WHERE origin=candidate.origin FOR UPDATE;
    now_at := clock_timestamp();
    IF action='commit' THEN
      SELECT * INTO stored FROM captures WHERE fetch_id=candidate.fetch_id;
      IF stored.fetch_id IS NOT NULL THEN
        IF stored.payload_sha256<>request->>'payload_sha256' OR stored.metadata IS DISTINCT FROM request->'metadata' THEN RETURN jsonb_build_object('version',2,'error','capture_conflict'); END IF;
        RETURN jsonb_build_object('version',2,'ack',true,'fetch_id',stored.fetch_id,'duplicate',true,'replicated',replicas>0);
      END IF;
    END IF;
    IF candidate.state<>'leased' OR authority.owner IS DISTINCT FROM worker
      OR authority.token IS DISTINCT FROM (request->>'token')::uuid
      OR authority.epoch IS DISTINCT FROM (request->>'epoch')::bigint OR authority.deadline IS NULL OR authority.deadline<=now_at THEN
      RETURN jsonb_build_object('version',2,'error','lease_fenced');
    END IF;
    IF action='renew' THEN
      IF NOT EXISTS(SELECT FROM partitions WHERE id=authority.partition_id AND owner=node) THEN
        RETURN jsonb_build_object('version',2,'error','partition_transferring');
      END IF;
      IF EXISTS(SELECT FROM auxiliary_origins a JOIN origins o USING(origin) JOIN partitions p ON p.id=o.partition_id WHERE a.fetch_id=candidate.fetch_id AND p.owner='__draining__') THEN RETURN jsonb_build_object('version',2,'error','partition_transferring'); END IF;
      UPDATE origins SET deadline=now_at+duration_ms*interval '1 millisecond' WHERE origin=authority.origin OR (token=authority.token AND origin IN (SELECT origin FROM auxiliary_origins WHERE fetch_id=candidate.fetch_id));
      UPDATE request_slots SET deadline=now_at+(duration_ms+cooldown_ms)*interval '1 millisecond' WHERE fetch_id=candidate.fetch_id;
      reply:=jsonb_build_object('version',2,'renewed',true,'lease_ms',duration_ms);
    ELSIF action='authorize' THEN
      DELETE FROM ip_slots WHERE deadline<now_at-interval '1 day';
      DELETE FROM budgets WHERE day<(now_at AT TIME ZONE 'UTC')::date-2;
      IF request->>'destination_origin'<>authority.origin THEN
        -- Page redirects hand off with their durable chain. Robots redirects
        -- may acquire an auxiliary, centrally fenced request lease. A draining
        -- partition or already active destination always defers the request.
        IF candidate.next_origin IS NOT NULL THEN RETURN jsonb_build_object('version',2,'error','redirect_origin_requires_lease','retry_after_ms',1); END IF;
        IF (SELECT count(*) FROM auxiliary_origins WHERE fetch_id=candidate.fetch_id)>=10 THEN RETURN jsonb_build_object('version',2,'error','redirect_limit'); END IF;
        item:=request->'destination';
        INSERT INTO origins(origin,partition_id) VALUES(item->>'origin',(item->>'partition_id')::integer) ON CONFLICT DO NOTHING;
        SELECT o.* INTO authority FROM origins o JOIN partitions p ON p.id=o.partition_id
          WHERE o.origin=request->>'destination_origin' AND p.owner<>'__draining__' FOR UPDATE OF o SKIP LOCKED;
        IF authority.origin IS NULL OR (authority.deadline+cooldown_ms*interval '1 millisecond'>now_at AND authority.token IS DISTINCT FROM (request->>'token')::uuid) THEN
          RETURN jsonb_build_object('version',2,'error','courtesy_deferred','retry_after_ms',1000);
        END IF;
        IF authority.token IS DISTINCT FROM (request->>'token')::uuid THEN
          UPDATE origins SET epoch=epoch+1,owner=worker,token=(request->>'token')::uuid,deadline=now_at+duration_ms*interval '1 millisecond' WHERE origin=authority.origin RETURNING * INTO authority;
          INSERT INTO auxiliary_origins VALUES(candidate.fetch_id,authority.origin,authority.token,authority.epoch) ON CONFLICT(fetch_id,origin) DO UPDATE SET token=excluded.token,epoch=excluded.epoch;
        END IF;
      END IF;
      IF authority.eligible_at>now_at THEN
        RETURN jsonb_build_object('version',2,'error','courtesy_deferred','retry_after_ms',ceil(extract(epoch FROM authority.eligible_at-now_at)*1000));
      END IF;
      PERFORM pg_advisory_xact_lock(781239442);
      DELETE FROM request_slots WHERE deadline<=now_at;
      IF NOT EXISTS(SELECT FROM request_slots WHERE fetch_id=candidate.fetch_id) AND (SELECT count(*) FROM request_slots)>=(request->>'global_concurrency')::integer THEN
        RETURN jsonb_build_object('version',2,'error','courtesy_deferred','retry_after_ms',1000);
      END IF;
      INSERT INTO request_slots VALUES(candidate.fetch_id,authority.deadline+cooldown_ms*interval '1 millisecond') ON CONFLICT(fetch_id) DO UPDATE SET deadline=excluded.deadline;
      -- Admission reserves worst-case body bytes before HTTP. Exhaustion defers
      -- durably until the next UTC day, never sleeps inside a worker.
      FOR scope_key IN SELECT key FROM (VALUES ('global'),('origin:' || authority.origin)) AS scopes(key) ORDER BY key LOOP
        INSERT INTO budgets(day,scope) VALUES((now_at AT TIME ZONE 'UTC')::date,scope_key) ON CONFLICT DO NOTHING;
        SELECT * INTO budget FROM budgets WHERE day=(now_at AT TIME ZONE 'UTC')::date AND scope=scope_key FOR UPDATE;
        quota := CASE WHEN scope_key='global' THEN (request->>'daily_requests')::bigint ELSE (request->>'origin_daily_requests')::bigint END;
        IF budget.requests>=quota OR (scope_key='global' AND budget.reserved_bytes+(request->>'maximum_payload_bytes')::bigint>(request->>'daily_bytes')::bigint) THEN
          UPDATE urls SET eligible_at=date_trunc('day',now_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'+interval '1 day',reason='aggregate_budget' WHERE id=candidate.id;
          RETURN jsonb_build_object('version',2,'error','aggregate_budget','retry_after_ms',ceil(extract(epoch FROM ((date_trunc('day',now_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC')+interval '1 day'-now_at))*1000));
        END IF;
      END LOOP;
      -- All IP locks use a deterministic order; no lock is held across HTTP.
      FOR address IN SELECT value FROM jsonb_array_elements_text(coalesce(request->'destination_ips','[]')) ORDER BY value LOOP
        PERFORM pg_advisory_xact_lock(hashtextextended('ip:' || address,0));
        IF (SELECT count(*) FROM ip_slots WHERE ip=address::inet AND deadline>now_at AND fetch_id<>candidate.fetch_id)>=(request->>'ip_concurrency')::integer THEN
          RETURN jsonb_build_object('version',2,'error','courtesy_deferred','retry_after_ms',1000);
        END IF;
      END LOOP;
      FOR address IN SELECT value FROM jsonb_array_elements_text(coalesce(request->'destination_ips','[]')) LOOP
        INSERT INTO ip_slots VALUES(candidate.fetch_id,address::inet,authority.deadline+cooldown_ms*interval '1 millisecond')
          ON CONFLICT(fetch_id,ip) DO UPDATE SET deadline=excluded.deadline;
      END LOOP;
      UPDATE budgets SET requests=requests+1,reserved_bytes=reserved_bytes+(request->>'maximum_payload_bytes')::bigint
        WHERE day=(now_at AT TIME ZONE 'UTC')::date AND scope IN ('global','origin:' || authority.origin);
      UPDATE urls SET http_attempts=http_attempts+1 WHERE id=candidate.id;
      UPDATE origins SET eligible_at=now_at+greatest((request->>'minimum_delay_ms')::bigint,coalesce((SELECT crawl_delay_ms FROM robots_cache WHERE origin=authority.origin AND expires_at>now_at),0))*interval '1 millisecond' WHERE origin=authority.origin;
      reply:=jsonb_build_object('version',2,'authorized',true);
    ELSIF action='robots_get' THEN
      SELECT jsonb_build_object('status',status,'text',body,'ttl_ms',greatest(1,floor(extract(epoch FROM expires_at-now_at)*1000)),'crawl_delay_ms',crawl_delay_ms) INTO item
        FROM robots_cache WHERE origin=request->>'cache_origin' AND expires_at>now_at;
      reply:=jsonb_build_object('version',2,'rules',item);
    ELSIF action='robots_put' THEN
      IF request->>'cache_origin'<>authority.origin THEN RETURN jsonb_build_object('version',2,'error','robots_origin_fenced'); END IF;
      DELETE FROM robots_cache WHERE expires_at<now_at;
      INSERT INTO robots_cache(origin,status,body,expires_at,crawl_delay_ms) VALUES(authority.origin,(request->'rules'->>'status')::integer,request->'rules'->>'text',now_at+least((request->'rules'->>'ttl_ms')::bigint,86400000)*interval '1 millisecond',coalesce((request->'rules'->>'crawl_delay_ms')::bigint,0))
        ON CONFLICT(origin) DO UPDATE SET status=excluded.status,body=excluded.body,expires_at=excluded.expires_at,crawl_delay_ms=excluded.crawl_delay_ms;
      UPDATE origins SET eligible_at=greatest(eligible_at,now_at+CASE WHEN (request->'rules'->>'status')::integer=429 OR (request->'rules'->>'status')::integer>=500 THEN (request->'rules'->>'ttl_ms')::bigint ELSE coalesce((request->'rules'->>'crawl_delay_ms')::bigint,0) END*interval '1 millisecond') WHERE origin=authority.origin;
      reply:=jsonb_build_object('version',2,'stored',true);
    ELSIF action='continue_redirect' THEN
      item:=request->'target';
      INSERT INTO origins(origin,partition_id) VALUES(item->>'origin',(item->>'partition_id')::integer) ON CONFLICT DO NOTHING;
      UPDATE urls SET continuation=request->'continuation',next_origin=item->>'origin' WHERE id=candidate.id;
      reply:=jsonb_build_object('version',2,'continued',true);
    ELSIF action='discover' THEN
      FOR item IN SELECT value FROM jsonb_array_elements(request->'urls') LOOP
        INSERT INTO origins(origin,partition_id) VALUES(item->>'origin',(item->>'partition_id')::integer) ON CONFLICT DO NOTHING;
        INSERT INTO urls(id,identity_version,url,origin,provenance) VALUES(item->>'id',1,item->>'url',item->>'origin',item->'provenance') ON CONFLICT DO NOTHING;
      END LOOP;
      reply:=jsonb_build_object('version',2,'discovered',jsonb_array_length(request->'urls'));
    ELSIF action='commit' THEN
      IF (request->'metadata'->>'schema_version')::integer<>1 OR request->'metadata'->>'fetch_id'<>candidate.fetch_id::text
        OR request->'metadata'->>'requested_url'<>candidate.url THEN RETURN jsonb_build_object('version',2,'error','capture_contract'); END IF;
      INSERT INTO captures(fetch_id,url_id,epoch,payload_sha256,payload,metadata) VALUES(candidate.fetch_id,candidate.id,authority.epoch,
        request->>'payload_sha256',decode(request->>'payload_hex','hex'),request->'metadata');
      INSERT INTO outbox(fetch_id,discoveries) VALUES(candidate.fetch_id,coalesce(request->'metadata'->'discovery_urls','[]'));
      UPDATE urls SET state='committed' WHERE id=candidate.id;
      UPDATE origins o SET owner=NULL,token=NULL,deadline=NULL,eligible_at=greatest(eligible_at,now_at+cooldown_ms*interval '1 millisecond')
        FROM auxiliary_origins a WHERE a.fetch_id=candidate.fetch_id AND a.origin=o.origin AND a.token=o.token AND a.epoch=o.epoch;
      DELETE FROM auxiliary_origins WHERE fetch_id=candidate.fetch_id;
      UPDATE origins SET owner=NULL,token=NULL,deadline=NULL,eligible_at=greatest(eligible_at,now_at+greatest(cooldown_ms,coalesce((request->>'origin_retry_after_ms')::bigint,0))*interval '1 millisecond') WHERE origin=authority.origin;
      UPDATE request_slots SET deadline=now_at+cooldown_ms*interval '1 millisecond' WHERE fetch_id=candidate.fetch_id;
      UPDATE ip_slots SET deadline=now_at+cooldown_ms*interval '1 millisecond' WHERE fetch_id=candidate.fetch_id;
      reply:=jsonb_build_object('version',2,'ack',true,'fetch_id',candidate.fetch_id,'duplicate',false,'replicated',replicas>0);
    ELSIF action='nack' THEN
      UPDATE urls SET state=CASE WHEN coalesce((request->>'permanent')::boolean,false) THEN 'terminal' ELSE 'queued' END,
        eligible_at=greatest(eligible_at,now_at+(request->>'retry_after_ms')::bigint*interval '1 millisecond'),reason=request->>'reason',
        attempts=attempts+CASE WHEN coalesce((request->>'turn_only')::boolean,false) THEN 0 ELSE 1 END,
        origin=coalesce(next_origin,origin),next_origin=NULL WHERE id=candidate.id;
      UPDATE origins o SET owner=NULL,token=NULL,deadline=NULL,eligible_at=greatest(eligible_at,now_at+cooldown_ms*interval '1 millisecond')
        FROM auxiliary_origins a WHERE a.fetch_id=candidate.fetch_id AND a.origin=o.origin AND a.token=o.token AND a.epoch=o.epoch;
      DELETE FROM auxiliary_origins WHERE fetch_id=candidate.fetch_id;
      UPDATE origins SET owner=NULL,token=NULL,deadline=NULL,eligible_at=greatest(eligible_at,now_at+greatest(cooldown_ms,coalesce((request->>'origin_retry_after_ms')::bigint,0))*interval '1 millisecond') WHERE origin=authority.origin;
      UPDATE request_slots SET deadline=now_at+cooldown_ms*interval '1 millisecond' WHERE fetch_id=candidate.fetch_id;
      UPDATE ip_slots SET deadline=now_at+cooldown_ms*interval '1 millisecond' WHERE fetch_id=candidate.fetch_id;
      reply:=jsonb_build_object('version',2,'nacked',true);
    ELSE RETURN jsonb_build_object('version',2,'error','unknown_action'); END IF;
  END IF;
  INSERT INTO commands(id,reply) VALUES((request->>'command_id')::uuid,reply);
  RETURN reply;
END $$;
