export const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export function normalize(s) {
 const active = new Set(s.keys.map(k=>k.site_id));
 s.priority = [...new Set([...s.priority.filter(id=>active.has(id)), ...active])];
 for (const v of Object.values(s.instances)) if(v.site_id && !active.has(v.site_id)) v.site_id='';
 return s;
}
export function move(s,id,delta) { const i=s.priority.indexOf(id), j=i+delta; if(i>=0 && j>=0 && j<s.priority.length) [s.priority[i],s.priority[j]]=[s.priority[j],s.priority[i]]; return s; }
