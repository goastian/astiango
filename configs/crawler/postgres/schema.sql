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
  IF action='publish_outbox' THEN
    SELECT * INTO publication FROM outbox WHERE fetch_id=(request->>'fetch_id')::uuid FOR UPDATE;
    IF publication.fetch_id IS NULL THEN RETURN jsonb_build_object('version',2,'error','unknown_outbox'); END IF;
    IF publication.published_at IS NOT NULL THEN RETURN jsonb_build_object('version',2,'published',0,'duplicate',true); END IF;
    IF publication.published_count IS DISTINCT FROM (request->>'cursor')::integer THEN
      RETURN jsonb_build_object('version',2,'error','outbox_changed');
    END IF;
    FOR item IN SELECT value FROM jsonb_array_elements(request->'urls') LOOP
      INSERT INTO origins(origin,partition_id) VALUES(item->>'origin',(item->>'partition_id')::integer) ON CONFLICT DO NOTHING;
      INSERT INTO urls(id,identity_version,url,origin,provenance) VALUES(item->>'id',1,item->>'url',item->>'origin',item->'provenance') ON CONFLICT DO NOTHING;
    END LOOP;
    UPDATE outbox SET published_count=(request->>'next_cursor')::integer,
      published_at=CASE WHEN (request->>'finished')::boolean THEN now_at ELSE NULL END WHERE fetch_id=publication.fetch_id;
    reply:=jsonb_build_object('version',2,'published',jsonb_array_length(request->'urls'),'finished',(request->>'finished')::boolean);
  ELSIF action='admit' THEN
    FOR item IN SELECT value FROM jsonb_array_elements(request->'urls') LOOP
      INSERT INTO origins(origin,partition_id) VALUES(item->>'origin',(item->>'partition_id')::integer) ON CONFLICT DO NOTHING;
      INSERT INTO urls(id,identity_version,url,origin,provenance) VALUES(item->>'id',1,item->>'url',item->>'origin',coalesce(item->'provenance','{}')) ON CONFLICT DO NOTHING;
    END LOOP;
    reply := jsonb_build_object('version',2,'admitted',jsonb_array_length(request->'urls'));
  ELSIF action='claim' THEN
    IF duration_ms IS NULL OR cooldown_ms IS NULL OR duration_ms NOT BETWEEN 100 AND 3600000 OR cooldown_ms NOT BETWEEN 0 AND 3600000 THEN
      RETURN jsonb_build_object('version',2,'error','invalid_lease_budget');
    END IF;
    SELECT u.* INTO candidate FROM urls u JOIN origins o ON o.origin=u.origin JOIN partitions p ON p.id=o.partition_id
      WHERE p.owner=node AND u.state IN ('queued','leased') AND u.eligible_at<=now_at AND o.eligible_at<=now_at
        AND (o.deadline IS NULL OR o.deadline + cooldown_ms * interval '1 millisecond' <= now_at)
      ORDER BY u.eligible_at,u.id FOR UPDATE OF u SKIP LOCKED LIMIT 1;
    IF candidate.id IS NULL THEN RETURN jsonb_build_object('version',2,'lease',NULL,'retry_after_ms',100); END IF;
    SELECT * INTO authority FROM origins WHERE origin=candidate.origin FOR UPDATE;
    now_at := clock_timestamp();
    IF authority.eligible_at>now_at OR authority.deadline + cooldown_ms * interval '1 millisecond'>now_at THEN
      RETURN jsonb_build_object('version',2,'lease',NULL,'retry_after_ms',100);
    END IF;
    UPDATE origins SET epoch=epoch+1,owner=worker,token=gen_random_uuid(),deadline=now_at+duration_ms*interval '1 millisecond' WHERE origin=candidate.origin RETURNING * INTO authority;
    UPDATE urls SET state='leased',attempts=attempts+1 WHERE id=candidate.id;
    reply := jsonb_build_object('version',2,'lease',jsonb_build_object('url',candidate.url,'url_id',candidate.id,'origin',candidate.origin,
      'fetch_id',candidate.fetch_id,'epoch',authority.epoch,'token',authority.token,'owner',worker,'attempts',candidate.attempts+1,'lease_ms',duration_ms,'server_now',now_at));
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
      UPDATE origins SET deadline=now_at+duration_ms*interval '1 millisecond' WHERE origin=authority.origin;
      reply:=jsonb_build_object('version',2,'renewed',true,'lease_ms',duration_ms);
    ELSIF action='authorize' THEN
      IF request->>'destination_origin'<>authority.origin THEN RETURN jsonb_build_object('version',2,'error','redirect_origin_requires_lease'); END IF;
      IF authority.eligible_at>now_at THEN
        RETURN jsonb_build_object('version',2,'error','courtesy_deferred','retry_after_ms',ceil(extract(epoch FROM authority.eligible_at-now_at)*1000));
      END IF;
      UPDATE origins SET eligible_at=now_at+(request->>'minimum_delay_ms')::bigint*interval '1 millisecond' WHERE origin=authority.origin;
      reply:=jsonb_build_object('version',2,'authorized',true);
    ELSIF action='commit' THEN
      IF (request->'metadata'->>'schema_version')::integer<>1 OR request->'metadata'->>'fetch_id'<>candidate.fetch_id::text
        OR request->'metadata'->>'requested_url'<>candidate.url THEN RETURN jsonb_build_object('version',2,'error','capture_contract'); END IF;
      INSERT INTO captures(fetch_id,url_id,epoch,payload_sha256,payload,metadata) VALUES(candidate.fetch_id,candidate.id,authority.epoch,
        request->>'payload_sha256',decode(request->>'payload_hex','hex'),request->'metadata');
      INSERT INTO outbox(fetch_id,discoveries) VALUES(candidate.fetch_id,coalesce(request->'metadata'->'discovery_urls','[]'));
      UPDATE urls SET state='committed' WHERE id=candidate.id;
      UPDATE origins SET owner=NULL,token=NULL,deadline=NULL,eligible_at=greatest(eligible_at,now_at+cooldown_ms*interval '1 millisecond') WHERE origin=authority.origin;
      reply:=jsonb_build_object('version',2,'ack',true,'fetch_id',candidate.fetch_id,'duplicate',false,'replicated',replicas>0);
    ELSIF action='nack' THEN
      UPDATE urls SET state=CASE WHEN coalesce((request->>'permanent')::boolean,false) THEN 'terminal' ELSE 'queued' END,
        eligible_at=now_at+(request->>'retry_after_ms')::bigint*interval '1 millisecond',reason=request->>'reason' WHERE id=candidate.id;
      UPDATE origins SET owner=NULL,token=NULL,deadline=NULL,eligible_at=greatest(eligible_at,now_at+cooldown_ms*interval '1 millisecond') WHERE origin=authority.origin;
      reply:=jsonb_build_object('version',2,'nacked',true);
    ELSE RETURN jsonb_build_object('version',2,'error','unknown_action'); END IF;
  END IF;
  INSERT INTO commands(id,reply) VALUES((request->>'command_id')::uuid,reply);
  RETURN reply;
END $$;
