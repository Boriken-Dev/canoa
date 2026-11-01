import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {execFileSync,spawnSync} from 'node:child_process';
import {WebSocket} from 'ws';
import {generateIdentity,createInitiator} from '../../../app/web/vendor/coqui-peer-webcrypto.mjs';
import {serve} from '../server.mjs';

// The fixture's TLS certificate comes from OpenSSL; without it the tests skip and say why.
const needs={skip:spawnSync('openssl',['version']).error?'openssl is not on PATH':false};

async function fixture(allow) {
  const dir=mkdtempSync(path.join(tmpdir(),'canoa-echo-'));
  const certFile=path.join(dir,'cert.pem'),keyFile=path.join(dir,'key.pem');
  execFileSync('openssl',['req','-x509','-newkey','rsa:2048','-nodes',
    '-keyout',keyFile,'-out',certFile,'-days','1','-subj','/CN=localhost',
    '-addext','subjectAltName=DNS:localhost,IP:127.0.0.1'],{stdio:'ignore'});
  const cert=readFileSync(certFile),key=readFileSync(keyFile);
  const client=await generateIdentity();
  const identity=await generateIdentity();
  const server=await serve({cert,key,identity,
    allowedPeerId: allow?client.peerId:'sha256:'+'0'.repeat(64)});
  const socket=new WebSocket('wss://127.0.0.1:'+server.port,{ca:cert});
  await new Promise((resolve,reject)=>{
    socket.once('open',resolve);socket.once('error',reject);
  });
  const messages=[];
  let consumer;
  socket.on('message',data=>{
    const frame=data;
    if(consumer){const take=consumer;consumer=null;take(frame);}
    else messages.push(frame);
  });
  function next(){
    if(messages.length)return Promise.resolve(messages.shift());
    return new Promise((resolve,reject)=>{
      const timeout=setTimeout(()=>reject(Error('Response timed out')),4000);
      consumer=frame=>{clearTimeout(timeout);resolve(frame);};
    });
  }
  return {client,server,socket,next,async close(){
    socket.terminate();
    await server.close();
    rmSync(dir,{recursive:true,force:true});
  }};
}
async function authenticate(f){
  const auth=await createInitiator(f.client,f.server.peerId);
  f.socket.send(auth.hello());
  const challenge=(await f.next()).toString();
  f.socket.send(await auth.answer(challenge));
  const accepted=(await f.next()).toString();
  return auth.finish(accepted);
}
test('actual WSS browser peer proof and ACP v2 prompt lifecycle',needs,async()=>{
  const f=await fixture(true);
  try{
    const {peerId,session}=await authenticate(f);
    assert.equal(peerId,f.server.peerId);
    const request=async(id,method,params)=>f.socket.send(await session.seal(JSON.stringify({
      jsonrpc:'2.0',id,method,params,
    })));
    const answer=async()=>JSON.parse(new TextDecoder().decode(await session.open(new Uint8Array(await f.next()))));
    await request(1,'initialize',{protocolVersion:2,capabilities:{},
      info:{name:'test-browser',version:'0.0.0'}});
    const init=await answer();
    assert.equal(init.id,1);
    assert.equal(init.result.protocolVersion,2);
    await request(2,'session/new',{cwd:'/',mcpServers:[]});
    const created=await answer();
    assert.equal(created.id,2);
    assert.ok(created.result.sessionId);
    const sessionId=created.result.sessionId;
    await request(3,'session/prompt',{sessionId,
      prompt:[{type:'text',text:'hello from browser'}]});
    const accepted=await answer();
    assert.equal(accepted.id,3);
    assert.ok(accepted.result.messageId);
    const events=[];
    for(let i=0;i<4;i++)events.push(await answer());
    assert.ok(events.every(x=>x.method==='session/update'&&x.params.sessionId===sessionId));
    assert.equal(events[0].params.update.messageId,accepted.result.messageId);
    assert.equal(events[1].params.update.state,'running');
    assert.equal(events[2].params.update.content.text,'Echo: hello from browser');
    assert.equal(events[3].params.update.stopReason,'end_turn');
  }finally{await f.close();}
});
test('real WSS refuses browser client missing from allowlist',needs,async()=>{
  const f=await fixture(false);
  try{
    const auth=await createInitiator(f.client,f.server.peerId);
    f.socket.send(auth.hello());
    f.socket.send(await auth.answer((await f.next()).toString()));
    const code=await new Promise(resolve=>f.socket.once('close',resolve));
    assert.equal(code,1008);
  }finally{await f.close();}
});
test('a plain message after a good proof ends the connection',needs,async()=>{
  const f=await fixture(true);
  try{
    await authenticate(f);
    f.socket.send(JSON.stringify({jsonrpc:'2.0',id:1,method:'initialize',params:{protocolVersion:2}}));
    const code=await new Promise(resolve=>f.socket.once('close',resolve));
    assert.equal(code,1008);
  }finally{await f.close();}
});
