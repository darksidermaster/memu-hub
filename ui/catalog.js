// Catálogo de sites: as opções de cadastro vêm de `ui/sites.json`.
//
// Formato:
//   { "CapSonic": { "icon": ".\\assets\\logos\\capsonic.svg" } }
//
// O `icon` pode ser um caminho relativo para uma imagem dentro de `ui/`
// (preferencialmente `assets/...`) ou um texto curto (até 2 caracteres).
// Nada de URL externa, caminho absoluto ou `..`: o valor é validado e escapado
// antes de ir para o HTML.

export const SITE_ICON_MAX_TEXT = 2;
export const CATALOG_FILE = 'sites.json';

function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, c => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[c]));
}

// `.\assets\logos\capsonic.svg` -> `assets/logos/capsonic.svg`
export function normalizeIconPath(raw) {
  const value = String(raw ?? '').trim();
  if (!value) return null;
  let path = value.replace(/\\/g, '/');
  while (path.startsWith('./')) path = path.slice(2);
  if (!path || path.startsWith('/') || path.includes('..') || path.includes('//')) return null;
  if (!/^[A-Za-z0-9._/-]+$/.test(path)) return null;
  if (!path.startsWith('assets/')) return null;
  return path;
}

// Devolve {type:'image', value} | {type:'text', value} | null.
export function normalizeIcon(raw) {
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) return normalizeIcon(raw.icon);
  const value = String(raw ?? '').trim();
  if (!value) return null;
  if ([...value].length <= SITE_ICON_MAX_TEXT && !/[\\/]/.test(value)) {
    return { type: 'text', value };
  }
  const path = normalizeIconPath(value);
  return path ? { type: 'image', value: path } : null;
}

export function slugify(name) {
  const base = String(name ?? '')
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return (base || 'site').slice(0, 80);
}

// Converte o JSON em uma lista estável, sem duplicar ids.
export function parseCatalog(data) {
  if (!data || typeof data !== 'object' || Array.isArray(data)) return [];
  const entries = [];
  const used = new Set();
  for (const [rawName, meta] of Object.entries(data)) {
    const name = String(rawName ?? '').trim();
    if (!name || name.length > 100) continue;
    let id = slugify(name);
    if (used.has(id)) {
      let suffix = 2;
      while (used.has(`${id}-${suffix}`)) suffix += 1;
      id = `${id}-${suffix}`;
    }
    used.add(id);
    const icon = normalizeIcon(meta);
    entries.push({ id, name, icon: icon ? icon.value : '', icon_type: icon ? icon.type : 'text' });
  }
  return entries;
}

export async function loadCatalog(fetchImpl = globalThis.fetch, url = CATALOG_FILE) {
  if (typeof fetchImpl !== 'function') throw new Error('Não foi possível carregar o catálogo de sites.');
  const response = await fetchImpl(url, { cache: 'no-store' });
  if (!response.ok) throw new Error(`Não foi possível ler ${url} (HTTP ${response.status}).`);
  const data = await response.json();
  const entries = parseCatalog(data);
  if (!entries.length) throw new Error(`${url} não contém nenhum site válido.`);
  return entries;
}

// Ícone de um site já salvo (texto legado ou caminho de imagem).
export function siteIconHtml(site) {
  const icon = normalizeIcon(site && site.icon);
  if (icon && icon.type === 'image') {
    return `<span class="site-icon"><img src="${escapeHtml(icon.value)}" alt=""></span>`;
  }
  const text = icon && icon.type === 'text'
    ? icon.value
    : (String((site && site.name) || '?').trim()[0] || '?');
  return `<span class="site-icon">${escapeHtml(text)}</span>`;
}

// Card do catálogo exibido em "Cadastrar site".
export function catalogCardHtml(entry, existing = false) {
  const icon = normalizeIcon(entry.icon);
  const inner = icon && icon.type === 'image'
    ? `<img src="${escapeHtml(icon.value)}" alt="">`
    : escapeHtml(icon && icon.value ? icon.value : (String(entry.name || '?')[0] || '?'));
  return `<button type="button" class="card row catalog-item" data-site="${escapeHtml(entry.id)}">`
    + `<span class="site-icon">${inner}</span>`
    + '<span class="grow catalog-name">'
    + `<strong>${escapeHtml(entry.name)}</strong>`
    + `<small>${existing ? 'API-key já cadastrada · toque para adicionar outra' : 'Toque para informar a API-key'}</small>`
    + '</span><span class="catalog-plus" aria-hidden="true">+</span></button>';
}
