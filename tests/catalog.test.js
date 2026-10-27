import test from 'node:test';
import assert from 'node:assert/strict';
import {
  parseCatalog,
  normalizeIcon,
  normalizeIconPath,
  slugify,
  siteIconHtml,
  catalogCardHtml,
  loadCatalog,
} from '../ui/catalog.js';

const CATALOG = {
  CapSonic: { icon: '.\\assets\\logos\\capsonic.svg' },
  NoneCap: { icon: '.\\assets\\logos\\nonecap.webp' },
};

test('parses the shipped sites.json', () => {
  const entries = parseCatalog(CATALOG);
  assert.equal(entries.length, 2);
  const capsonic = entries.find(e => e.id === 'capsonic');
  assert.equal(capsonic.name, 'CapSonic');
  assert.equal(capsonic.icon_type, 'image');
  assert.equal(capsonic.icon, 'assets/logos/capsonic.svg');
  assert.equal(entries.find(e => e.id === 'nonecap').icon, 'assets/logos/nonecap.webp');
});

test('normalizes Windows backslashes and leading ./', () => {
  assert.equal(normalizeIconPath('.\\assets\\logos\\capsonic.svg'), 'assets/logos/capsonic.svg');
  assert.equal(normalizeIconPath('./assets/logos/x.webp'), 'assets/logos/x.webp');
  assert.equal(normalizeIconPath('assets/logos/x.png'), 'assets/logos/x.png');
});

test('rejects unsafe icon paths', () => {
  for (const bad of ['..\\secrets.png', 'C:\\Windows\\x.png', '/etc/passwd', 'https://x/y.png', 'assets/../../x', 'assets//x.png', 'assets/logo x.png', '']) {
    assert.equal(normalizeIconPath(bad), null, `deveria rejeitar ${bad}`);
  }
});

test('accepts short text icons and rejects long ones', () => {
  assert.deepEqual(normalizeIcon('C'), { type: 'text', value: 'C' });
  assert.deepEqual(normalizeIcon('AB'), { type: 'text', value: 'AB' });
  assert.equal(normalizeIcon('ABC'), null);
  assert.equal(normalizeIcon('A/B'), null);
});

test('slugify produces stable safe ids', () => {
  assert.equal(slugify('CapSonic'), 'capsonic');
  assert.equal(slugify('None Cap'), 'none-cap');
  assert.equal(slugify('Solução Ágil'), 'solucao-agil');
  assert.equal(slugify('???'), 'site');
});

test('duplicate slugs get suffixes', () => {
  const entries = parseCatalog({ 'Site A': { icon: 'A' }, 'site a': { icon: 'B' } });
  assert.deepEqual(entries.map(e => e.id), ['site-a', 'site-a-2']);
});

test('ignores invalid catalog shapes and entries', () => {
  assert.deepEqual(parseCatalog(null), []);
  assert.deepEqual(parseCatalog([]), []);
  assert.deepEqual(parseCatalog('x'), []);
  const entries = parseCatalog({ '  ': { icon: 'A' }, Ok: { icon: 'O' } });
  assert.deepEqual(entries.map(e => e.id), ['ok']);
});

test('catalogCardHtml renders the image and escapes the name', () => {
  const html = catalogCardHtml({ id: 'capsonic', name: '<b>Cap</b>', icon: 'assets/logos/capsonic.svg' });
  assert.ok(html.includes('src="assets/logos/capsonic.svg"'));
  assert.ok(!html.includes('<b>Cap</b>'));
  assert.ok(html.includes('&lt;b&gt;Cap&lt;/b&gt;'));
  assert.ok(html.includes('data-site="capsonic"'));
});

test('catalogCardHtml marks sites that already have a key', () => {
  const html = catalogCardHtml({ id: 'a', name: 'A', icon: 'A' }, true);
  assert.match(html, /já cadastrada/);
});

test('siteIconHtml falls back to the first letter', () => {
  assert.ok(siteIconHtml({ name: 'CapSonic', icon: '' }).includes('C'));
  assert.ok(siteIconHtml({ name: 'X', icon: 'assets/logos/capsonic.svg' }).includes('<img'));
  assert.ok(siteIconHtml({ name: 'X', icon: '..\\evil.png' }).includes('X'));
});

test('loadCatalog reads and validates the JSON', async () => {
  const entries = await loadCatalog(async () => ({ ok: true, json: async () => CATALOG }), 'sites.json');
  assert.equal(entries.length, 2);
});

test('loadCatalog surfaces HTTP errors', async () => {
  await assert.rejects(
    loadCatalog(async () => ({ ok: false, status: 404 }), 'sites.json'),
    /HTTP 404/,
  );
});

test('loadCatalog rejects an empty catalog', async () => {
  await assert.rejects(
    loadCatalog(async () => ({ ok: true, json: async () => ({}) }), 'sites.json'),
    /nenhum site válido/,
  );
});
