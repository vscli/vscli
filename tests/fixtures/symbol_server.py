#!/usr/bin/env python3
"""Native symbol protocol fixture; canceled requests deliberately still reply."""
import json, sys, threading, time
lock = threading.Lock()
root = None
count = 0

def send(message):
    data=json.dumps({'jsonrpc':'2.0', **message}).encode()
    with lock:
        sys.stdout.buffer.write(f'Content-Length: {len(data)}\r\n\r\n'.encode()+data)
        sys.stdout.buffer.flush()

def delayed(ident, result, delay=.08):
    def run():
        time.sleep(delay)
        send({'id':ident,'result':result})
    threading.Thread(target=run,daemon=True).start()

def span(start=2, end=4):
    return {'start':{'line':0,'character':start}, 'end':{'line':0,'character':end}}

while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,value=line.decode().split(':',1);headers[key.lower()]=value.strip()
    msg=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method, ident, params=msg.get('method'),msg.get('id'),msg.get('params',{})
    if method=='initialize':
        root=params['rootUri']
        send({'id':ident,'result':{'capabilities':{'textDocumentSync':1,'documentSymbolProvider':True,'workspaceSymbolProvider':True}}})
    elif method=='textDocument/documentSymbol':
        assert set(params)=={'textDocument'}
        child={'name':'child 😀','kind':12,'range':span(0,7),'selectionRange':span()}
        parent={'name':'parent','kind':5,'range':span(0,7),'selectionRange':span(0,1),'children':[child]}
        delayed(ident,[parent])
    elif method=='workspace/symbol':
        count+=1;query=params['query']
        path='other.cpp' if query not in ('missing','invalid','fifo') else query+'.cpp'
        symbol={'name':query or 'other','kind':12,'containerName':'fixture','location':{'uri':root+'/'+path,'range':span()}}
        if query=='invalid': symbol['location']['range']=span(3,4)
        if query=='unresolved': symbol['location'].pop('range')
        if query=='nonfile': symbol['location']['uri']='untitled:example'
        result=[symbol]*513 if query=='oversized' else [symbol]
        delayed(ident,result,.35 if query=='slow' else .04)
    elif method=='shutdown': send({'id':ident,'result':None})
    elif method=='exit': break
