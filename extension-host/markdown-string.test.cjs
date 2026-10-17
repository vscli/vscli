'use strict';
const test=require('node:test'), assert=require('node:assert/strict');
const {MarkdownString:M}=require('./markdown-string.cjs');
const types=require('./provider-types.cjs');
const {Uri,Range,TextDocument}=require('./api-types.cjs');
const {normalize:provider}=require('./providers.cjs');
const {normalize:signature}=require('./signatures.cjs');
const {normalize:diagnostic}=require('./diagnostics.cjs');
const document=()=>new TextDocument({id:1,uri:'untitled:markdown',version:1,languageId:'cpp',text:'猫🙂\r\nvalue',isDirty:true});
const help=docs=>({activeSignature:0,activeParameter:0,signatures:[{label:'f()',documentation:docs,parameters:[]}]});

test('single exported class uses pinned defaults and private delegate prototype accessors',()=>{
  assert.equal(M,types.MarkdownString);
  const value=new M(); assert.equal(value.value,''); assert.equal(value.isTrusted,undefined);
  assert.equal(value.supportThemeIcons,false); assert.equal(value.supportHtml,false); assert.equal(value.supportAlertSyntax,false); assert.equal(value.baseUri,undefined);
  assert.deepEqual(Object.keys(value),[]); assert.equal(JSON.stringify(value),'{}');
  for(const key of ['value','isTrusted','supportThemeIcons','supportHtml','supportAlertSyntax','baseUri']) {
    assert.equal(Object.hasOwn(value,key),false); const descriptor=Object.getOwnPropertyDescriptor(M.prototype,key);
    assert.equal(typeof descriptor.get,'function'); assert.equal(typeof descriptor.set,'function'); assert.equal(descriptor.enumerable,false); assert.equal(descriptor.configurable,true);
  }
});

test('constructor enforces only pinned string type and allows large public values',()=>{
  for(const value of [null,17,{},Symbol('bad'),new String('wrapped')]) assert.throws(()=>new M(value),{name:'Error',message:'Illegal argument: value'});
  assert.equal(new M(undefined).value,''); const text='x'.repeat(65537); assert.equal(new M(text).value,text);
  assert.equal(new M('x',null).supportThemeIcons,false); assert.equal(new M('x',17).supportThemeIcons,17);
});

test('setters preserve trust, URI and arbitrary value references without native side effects',()=>{
  const value=new M(), trust={enabledCommands:['command.run']}, uri=Uri.parse('file:///folder/');
  value.isTrusted=trust; value.baseUri=uri; value.supportHtml=true; value.supportThemeIcons=true; value.supportAlertSyntax=true;
  assert.equal(value.isTrusted,trust); assert.equal(value.baseUri,uri); assert(value.supportHtml); assert(value.supportThemeIcons); assert(value.supportAlertSyntax);
  const opaque={toString:()=> 'old'}; value.value=opaque; assert.equal(value.value,opaque);
  assert.equal(value.appendMarkdown('new'),value); assert.equal(value.value,'oldnew');
});

test('appendText escapes pinned tokens and preserves nonleading hyphens, periods and angle opener',()=>{
  const value=new M(); assert.equal(value.appendText('a.b - c <d> ~e~\n - bullet\tz'),value);
  assert.equal(value.value,'a.b&nbsp;-&nbsp;c&nbsp;<d\\>&nbsp;\\~e\\~\n\n&nbsp;\\-&nbsp;bullet&nbsp;z');
  assert.equal(new M().appendText('`*_{}[]()#+!\\').value,'\\`\\*\\_\\{\\}\\[\\]\\(\\)\\#\\+\\!\\\\');
});

test('public appendText doubles LF, retains CR and ignores nonpublic newline-style arguments',()=>{
  const value=new M().appendText('猫\r\n🙂\nend',1);
  assert.equal(value.value,'猫\r\n\n🙂\n\nend');
  assert.throws(()=>new M().appendText(17),TypeError);
});

test('theme icon escaping follows pinned valid-name/modifier grammar and escaped icon handling',()=>{
  assert.equal(new M('',true).appendText('$(zap)').value,'\\\\$\\(zap\\)');
  assert.equal(new M('',true).appendText('\\$(zap)').value,'\\\\$\\(zap\\)');
  assert.equal(new M('',false).appendText('$(zap)').value,'$\\(zap\\)');
  assert.equal(new M('',true).appendText('$(sync~spin)').value,'\\\\$\\(sync\\~spin\\)');
  assert.equal(new M('',true).appendText('$(bad_name)').value,'$\\(bad\\_name\\)');
});

test('appendMarkdown preserves ordinary plus coercion and returns the original value',()=>{
  const value=new M('prefix'); assert.equal(value.appendMarkdown({toString:()=> '*raw*'}),value);
  assert.equal(value.value,'prefix*raw*'); value.appendMarkdown(null); assert.equal(value.value,'prefix*raw*null');
  assert.throws(()=>value.appendMarkdown(Symbol('bad')),TypeError);
});

test('appendCodeblock retains code bytes and uses a longer fence than leading backtick runs',()=>{
  const code='猫\r\n```inner\r\n`````tail\r\n🙂';
  const value=new M('prefix'); assert.equal(value.appendCodeblock(code,'cpp'),value);
  assert.equal(value.value,'prefix\n``````cpp\n'+code+'\n``````\n');
  assert.equal(new M().appendCodeblock('x ``` interior','json').value,'\n```json\nx ``` interior\n```\n');
});

test('codeblock defaults language with nullish semantics and retains source validation errors',()=>{
  assert.equal(new M().appendCodeblock('x').value,'\n```\nx\n```\n');
  assert.equal(new M().appendCodeblock('x',null).value,'\n```\nx\n```\n');
  assert.equal(new M().appendCodeblock('x',17).value,'\n```17\nx\n```\n');
  assert.throws(()=>new M().appendCodeblock(17,'cpp'),TypeError);
});

test('source static recognition uses exact instance-or-duck semantics without fabricated type checks',()=>{
  assert.equal(M.isMarkdownString(new M()),true); assert.equal(M.isMarkdownString(null),false); assert.equal(M.isMarkdownString('x'),false);
  const shaped={appendCodeblock:1,appendMarkdown:2,appendText:3,value:null}; assert.equal(M.isMarkdownString(shaped),true);
  shaped.appendMarkdown=0; assert.equal(M.isMarkdownString(shaped),0);
  assert.equal(M.isMarkdownString({appendCodeblock(){},appendMarkdown(){},appendText(){}}),false);
  const value=new M(); value.value=undefined; assert.equal(M.isMarkdownString(value),true);
});

test('private delegation survives overridden public accessors and rejects borrowed receivers',()=>{
  class Override extends M { get value(){return 'outer';} set value(_value){throw new Error('must not set outer');} }
  const value=new Override('inner'); value.appendText(' x');
  assert.equal(value.value,'outer'); assert.equal(Object.getOwnPropertyDescriptor(M.prototype,'value').get.call(value),'inner&nbsp;x');
  assert.throws(()=>M.prototype.appendMarkdown.call({},'x'),TypeError);
});

test('shared ES5 helper preserves legacy subclass identity and private delegate brand',()=>{
  function Legacy(value,icons){return M.apply(this,[value,icons]);}
  Legacy.prototype=Object.create(M.prototype); Legacy.prototype.constructor=Legacy;
  const value=new Legacy('x',true); assert(value instanceof Legacy); assert(value instanceof M);
  assert.equal(value.appendText(' $(zap)'),value); assert.equal(value.value,'x&nbsp;\\\\$\\(zap\\)');
});

test('public trust/HTML values remain legal but native hover and completion boundaries reject them',()=>{
  const doc=document();
  for(const flag of ['isTrusted','supportHtml']) {
    const value=new M('[Run](command:run)<b>text</b>'); value[flag]=flag==='isTrusted'?{enabledCommands:['run']}:true;
    assert.throws(()=>provider('hover',new types.Hover(value),doc),/Trusted or HTML/);
    const item=new types.CompletionItem('x'); item.documentation=value;
    assert.throws(()=>provider('completion',[item],doc),/Trusted or HTML/);
  }
  const plain=new M('**safe**'); assert.deepEqual(provider('hover',new types.Hover(plain),doc).contents,[{kind:'markdown',value:'**safe**'}]);
});

test('native provider documentation limits apply after unrestricted construction without truncating values',()=>{
  const doc=document(), value=new M('x'.repeat(65537));
  assert.throws(()=>provider('hover',new types.Hover(value),doc),/budget/); assert.equal(value.value.length,65537);
  value.value='🙂'.repeat(16385); assert.throws(()=>provider('hover',new types.Hover(value),doc),/budget/);
  value.value='🙂'.repeat(16384); assert.equal(provider('hover',new types.Hover(value),doc).contents[0].value,value.value);
  assert.equal(doc.getText(),'猫🙂\r\nvalue'); assert.equal(doc.version,1); assert(doc.isDirty);
});

test('native signature docs preserve strict trust/HTML and 8KiB limits',()=>{
  const value=new M('x'); value.isTrusted=true; assert.throws(()=>signature(help(value)),/Trusted or HTML/);
  value.isTrusted=undefined; value.supportHtml=true; assert.throws(()=>signature(help(value)),/Trusted or HTML/);
  value.supportHtml=false; value.value='🙂'.repeat(2049); assert.throws(()=>signature(help(value)),/budget/);
  value.value='🙂'.repeat(2048); assert.equal(signature(help(value)).signatures[0].documentation.value,value.value);
});

test('diagnostic messages stay bounded plain strings and never implicitly convert MarkdownString',()=>{
  const doc=document(), range=new Range(0,0,0,1), value=new M('**message**');
  assert.throws(()=>diagnostic({range,message:value,severity:0},doc),/Diagnostic text/);
  assert.throws(()=>diagnostic({range,message:'x'.repeat(4097),severity:0},doc),/Diagnostic text/);
  assert.equal(diagnostic({range,message:'猫🙂',severity:0},doc).message,'猫🙂');
  assert.equal(doc.getText(),'猫🙂\r\nvalue'); assert.equal(doc.version,1); assert(doc.isDirty);
});

test('native cheap length admission rejects oversized input before UTF8 measurement',()=>{
  const measure=Buffer.byteLength; let measured=false;
  const hover=new types.Hover(new M('x'.repeat(65537))), range=new Range(0,0,0,1);
  try {
    Buffer.byteLength=()=>{measured=true; throw new Error('must not measure rejected input');};
    assert.throws(()=>provider('hover',hover,document()),/budget/); assert.equal(measured,false);
    assert.throws(()=>signature({signatures:[{label:'x'.repeat(8193),parameters:[]}],activeSignature:0,activeParameter:0}),/budget/); assert.equal(measured,false);
    assert.throws(()=>diagnostic({range,message:'x'.repeat(4097),severity:0},document()),/Diagnostic text/); assert.equal(measured,false);
  } finally {Buffer.byteLength=measure;}
});
