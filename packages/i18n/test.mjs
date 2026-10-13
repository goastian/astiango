import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { build } from '../../frontend/node_modules/esbuild/lib/main.js';

const directory = await mkdtemp(join(tmpdir(), 'astiango-i18n-'));
await build({ entryPoints: [new URL('./index.ts', import.meta.url).pathname], outfile: join(directory, 'i18n.mjs'), bundle: true, platform: 'node', format: 'esm', logLevel: 'silent' });
const { languages, loadLocale, translate, resolveLocale, localizedHref } = await import(pathToFileURL(join(directory, 'i18n.mjs')).href);
await rm(directory, { recursive: true, force: true });
const catalogs = Object.fromEntries(await Promise.all(languages.map(async ({ code }) => [code, JSON.parse(await readFile(new URL(`./messages/${code}.json`, import.meta.url), 'utf8'))])));

test('all seven catalogs cover the same messages and preserve interpolation parameters', () => {
  const keys = Object.keys(catalogs.en).sort();
  assert.ok(keys.length > 1_100);
  for (const { code } of languages) {
    assert.deepEqual(Object.keys(catalogs[code]).sort(), keys, code);
    for (const token of ['GET', 'POST', '© 2026 Astian', 'astian.org / midori-browser']) assert.equal(catalogs[code][token], token, `${code}: ${token}`);
    for (const key of ['Add optic', 'No Optic', 'Optic selector', 'Manage Optics']) assert.match(catalogs[code][key], /Optic/, `${code}: ${key}`);
    for (const key of keys) {
      const value = catalogs[code][key];
      assert.ok(value.trim(), `${code}: ${key}`);
      assert.ok(!/ZXQPH|ZXQKEEP/.test(value), `${code}: ${key}`);
      assert.deepEqual(value.match(/\{p\d+\}/g)?.sort() ?? [], key.match(/\{p\d+\}/g)?.sort() ?? [], `${code}: ${key}`);
    }
  }
  assert.equal(catalogs['zh-CN'].Auto, '自动');
  assert.equal(catalogs['zh-CN'].Light, '浅色');
});

test('language aliases and unsupported or malformed preferences resolve safely', () => {
  for (const [input, expected] of [['de-DE','de'],['pt-BR','pt'],['zh-Hans','zh-CN'],['zh_CN','zh-CN'],['EN-us','en'],['es-CO','es'],['unknown','en'],[null,'en'],['<script>','en']]) assert.equal(resolveLocale(input), expected);
});

test('parameters are substituted once and unknown content is preserved', async () => {
  await Promise.all(languages.map(({ code }) => loadLocale(code)));
  for (const { code } of languages) {
    assert.equal(translate(code, 'https://example.org/?sig=abc&x=2'), 'https://example.org/?sig=abc&x=2');
    assert.equal(translate(code, '{p0} · AstianGO', { p0: '{p1} <script>' }), '{p1} <script> · AstianGO');
    assert.equal(translate(code, 'Vista general'), catalogs[code]['Vista general']);
  }
});

test('navigation preserves paths, query values, fragments and external links', () => {
  assert.equal(localizedHref('/search?q=a%26b&gl=France#results','de'), '/search?q=a%26b&gl=France&ui_lang=de#results');
  assert.equal(localizedHref('/settings?ui_lang=es','fr'), '/settings?ui_lang=fr');
  for (const href of ['https://example.org/?sig=a%2Fb','//example.org/path','#main','mailto:contact@astian.org']) assert.equal(localizedHref(href,'zh-CN'),href);
});

test('local CJK font subsets cover all owned Chinese interface and article text', async () => {
  const manifest = JSON.parse(await readFile(new URL('./fonts/manifest.json',import.meta.url),'utf8'));
  const covered = new Set(manifest.subsets.flatMap(subset => subset.codepoints));
  let text = Object.values(catalogs['zh-CN']).join('');
  for(const article of ['about','webmasters','privacy-and-happy-lawyers']) text += await readFile(new URL(`../../frontend/src/lib/articles/${article}/zh-CN.md`,import.meta.url),'utf8');
  for(const char of text) if(char.codePointAt(0)>=0x3000 && char.codePointAt(0)<=0xffff) assert.ok(covered.has(char.codePointAt(0)),`Uncovered Chinese glyph: ${char}`);
  const { createHash } = await import('node:crypto');
  for(const subset of manifest.subsets){const data=await readFile(new URL(`./fonts/${subset.file}`,import.meta.url));assert.equal(data.subarray(0,4).toString(),'wOF2');assert.equal(createHash('sha256').update(data).digest('hex'),subset.sha256);}
});
