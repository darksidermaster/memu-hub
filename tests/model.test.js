import test from 'node:test';
import assert from 'node:assert/strict';
import {esc, normalize, move} from '../ui/model.js';
test('removing last key clears priority and instance preference',()=>{
 const s={keys:[{site_id:'b'}],priority:['a','b'],instances:{0:{site_id:'a'}}};
 normalize(s);assert.deepEqual(s.priority,['b']);assert.equal(s.instances[0].site_id,'');
});
test('multiple keys for a site produce a single priority',()=>{
 const s={keys:[{site_id:'a'},{site_id:'a'},{site_id:'b'}],priority:[],instances:{}};
 normalize(s);assert.deepEqual(s.priority,['a','b']);move(s,'b',-1);assert.deepEqual(s.priority,['b','a']);move(s,'b',-1);assert.deepEqual(s.priority,['b','a']);
});
test('untrusted names are escaped',()=>{assert.equal(esc('<img src="x" onerror=\'a\'>'),'&lt;img src=&quot;x&quot; onerror=&#39;a&#39;&gt;')});
