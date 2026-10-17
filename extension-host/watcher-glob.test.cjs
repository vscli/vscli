'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { compileGlob } = require('./watcher-glob.cjs');
function matches(pattern, yes, no, options) {
  const match = compileGlob(pattern, options);
  for (const value of yes) assert.equal(match(value),true,JSON.stringify({pattern,value,expected:true}));
  for (const value of no) assert.equal(match(value),false,JSON.stringify({pattern,value,expected:false}));
}
test('single stars and question marks match path segments, never implicit basenames',()=>{
  matches('*.rs',['a.rs','.rs','猫.rs'],['/workspace/a.rs','sub/a.rs','sub\\a.rs','a.cpp']);
  matches('src/a?.rs',['src/ab.rs','src/a猫.rs'],['src/a.rs','src/a🙂.rs','src/a/b.rs','src/a\\b.rs','other/ab.rs']);
  matches('src/*/a.rs',['src/sub/a.rs','src//a.rs','src\\sub\\a.rs'],['src/sub/deeper/a.rs','src/a.rs']);
});
test('globstars include zero, one and many segments with dual path separators',()=>{
  matches('**/*.rs',['a.rs','/workspace/a.rs','/workspace/sub/a.rs','C:\\workspace\\a.rs'],['a.cpp','a.rs/child']);
  matches('src/**/a?.rs',['src/ab.rs','src/sub/ab.rs','src/sub/deep/a猫.rs','src\\sub\\ab.rs'],['src/a.rs','src/sub/a/b.rs','source/ab.rs']);
  matches('**/foo/**',['foo','foo/','foo/a/b','/workspace/foo','/workspace/foo/sub','C:\\workspace\\foo\\sub'],['foobar','/workspace/foobar','fooextra/a']);
  matches('**/**/a?.rs',['ab.rs','/workspace/ab.rs','C:\\workspace\\sub\\ab.rs'],['a.rs','ab.cpp']);
});
test('all-globstar special case preserves pinned dot-regexp newline behavior',()=>{
  for(const pattern of ['**','**/**','**/**/**'])matches(pattern,['','a','/a/b','C:\\a\\b'],['a\nb']);
  matches('*',['a\nb',''],['/a','a\\b']);
});
test('brace choices preserve independent wildcard and full-path alternatives',()=>{
  matches('**/*.{rs,cpp}',['a.rs','a.cpp','/workspace/a.rs','C:\\workspace\\a.cpp'],['a.c','a.rsx','a.cpp/child']);
  matches('{src/**/a?.rs,other/*.cpp}',['src/ab.rs','src/deep/ab.rs','other/a.cpp'],['src/a.rs','other/deep/a.cpp','/other/a.cpp']);
  matches('**/{foo,bar}/?.[ch]',['foo/a.c','/root/bar/x.h'],['foo/deep/a.c','baz/a.c','bar/a.cpp']);
  matches('{a/b,c/d}',['a/b','c/d','a\\b'],['/a/b','a/d','a/b/child']);
});
test('basename/extension optimizations retain full fs paths and exact suffix boundaries',()=>{
  matches('**/package.json',['package.json','/root/package.json','C:\\root\\package.json'],['not-package.json','/root/not-package.json','package.json/a']);
  matches('{**/package.json,**/Cargo.toml,**/*.rs}',['/root/package.json','C:\\root\\Cargo.toml','/root/猫.rs'],['/root/not-package.json','Cargo.toml/a','a.rs.backup']);
  matches('**/*.test.js',['.test.js','/root/a.test.js'],['test.js','/root/test.js','a.test.jsx']);
});
test('path fast paths preserve native/POSIX separator rules and do not match unrelated prefixes',()=>{
  const native=path.sep==='\\';
  matches('**/foo/bar',['foo/bar','/root/foo/bar',...(native?['C:\\root\\foo\\bar']:[])],['foobar','/root/xfoo/bar','/root/foo/bar2',...(!native?['C:\\root\\foo\\bar']:[])]);
  matches('foo/bar',['foo/bar',...(native?['foo\\bar']:[])],['/root/foo/bar','foo/bar/child',...(!native?['foo\\bar']:[])]);
  matches('**/foo/bar/',['foo/bar','/root/foo/bar'],['foo/bar/']);
});
test('bracket classes support ranges, both negation operators and first literal closing bracket',()=>{
  matches('file.[0-9]',['file.0','file.9'],['file.a','file.10']);
  matches('file.[!0-9]',['file.a','file._'],['file.0','file.9']);
  matches('file.[^ab]',['file.c'],['file.a','file.b']);
  matches('file.[]a]',['file.]','file.a'],['file.b']);
  matches('file.[-a]',['file.-','file.a'],['file.b']);
  matches('file.[+.$]',['file.+','file..','file.$'],['file.a']);
  matches('file.[a/]',['file.a'],['file./','file.b']);
  // Source drops explicit slash from a range; a negated range remains a
  // negated JS class, including its historical ability to match a separator.
  matches('[!a]',['b','/','\\'],['a']);
});
test('regex metacharacters outside glob constructs remain literal',()=>{
  matches('a.+(b)^$|c',['a.+(b)^$|c'],['a.zzb','a.+b^$|c']);
  matches('a\\b',['a\\b'],['a/b','ab']);
  matches('**/猫?.rs',['猫a.rs','/workspace/猫b.rs'],['猫.rs','猫ab.rs']);
});
test('case-sensitive default and ignoreCase cover fast paths, braces and regex',()=>{
  matches('**/package.json',['/root/package.json'],['/root/PACKAGE.JSON']);
  matches('**/package.json',['/root/PACKAGE.JSON','C:\\root\\Package.Json'],[],{ignoreCase:true});
  matches('**/*.rs',['/root/FILE.RS'],['/root/file.rsx'],{ignoreCase:true});
  matches('{**/Cargo.toml,**/*.RS}',['/root/cargo.TOML','/root/a.rs'],['a.cpp'],{ignoreCase:true});
  matches('src/**/a?.[ch]',['SRC/DEEP/Ab.C'],['SRC/Ab.cpp'],{ignoreCase:true});
  matches('**/i',['/root/İ'],[],{ignoreCase:true}); // strings.ts retains original UTF-16 substring bounds
});
test('trimming, malformed patterns and invalid classes preserve source outcomes',()=>{
  matches('  **/*.rs  ',['/root/a.rs'],['a.cpp']);
  matches('',[],['','a']);matches('   ',[''],['a']);
  matches('foo{bar',['foo'],['foobar','foo{bar']);
  matches('foo[bar',['foo'],['foob','foo[bar']);
  matches('x[z-a]',[],['xz','xa']);
  matches('{a,}',['a'],['']);matches('{,a}',['','a'],['b']);
  matches('{{a}}',[''],['a']); // upstream braces are stateful, not a balanced-brace dialect
});
test('compiled functions are reusable and independent; non-string paths never coerce',()=>{
  const rs=compileGlob('**/*.rs'),cpp=compileGlob('**/*.cpp');
  for(let i=0;i<100;i++){assert.equal(rs('/root/a.rs'),true);assert.equal(rs('/root/a.cpp'),false);assert.equal(cpp('/root/a.cpp'),true);}
  assert.equal(rs({toString(){throw new Error('must not coerce');}}),false);assert.equal(rs(undefined),false);
});
test('admission and expansion guards refuse explicitly without changing valid neighboring matchers',()=>{
  const valid=compileGlob('**/*.rs');
  for(const pattern of ['x'.repeat(4097),'猫'.repeat(1400),'a\0b'])assert.throws(()=>compileGlob(pattern),/4096|NUL/);
  assert.throws(()=>compileGlob(7),/string/);assert.throws(()=>compileGlob('*',{ignoreCase:'yes'}),/boolean/);
  // Refuse excessive choice expansion without replacing the admitted
  // semantics by an always-false or broader matcher.
  assert.throws(()=>compileGlob('{'+new Array(600).fill('a').join(',')+'}'),/expansion\/work/);
  assert.equal(valid('/root/a.rs'),true);assert.equal(valid('/root/a.cpp'),false);
});
