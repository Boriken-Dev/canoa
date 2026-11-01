// Reference ACP v2 echo fixture over WSS + Coqui peer proof. NOT a production agent.
import https from 'node:https';
import {readFileSync} from 'node:fs';
import {randomUUID, createPrivateKey} from 'node:crypto';
import {WebSocketServer} from 'ws';
import {createResponder, generateIdentity, importIdentity} from '../../app/web/vendor/coqui-peer-webcrypto.mjs';

const validPeer = /^sha256:[0-9a-f]{64}$/;
export async function loadIdentity(keyFile) {
  if (!keyFile) return generateIdentity();
  return importIdentity(createPrivateKey(readFileSync(keyFile)).export({format:'der', type:'pkcs8'}));
}
export async function serve({cert,key,identity,allowedPeerId,host='127.0.0.1',port=0}) {
  if (!validPeer.test(allowedPeerId || '') || !identity?.privateKey) {
    throw Error('Explicit paired client identity and server signing key are required');
  }
  const httpsServer = https.createServer({cert,key}, (_req,res)=>{
    res.writeHead(404); res.end();
  });
  const sockets = new Set();
  const wss = new WebSocketServer({server:httpsServer,maxPayload:1048576});
  wss.on('connection',socket=>{
    sockets.add(socket);
    socket.on('close',()=>sockets.delete(socket));
    const sessions = new Map();
    let state='hello', auth, session, initialized=false;
    let chain=Promise.resolve();
    // Sealed in call order, and sent in that order: each is one record in one binary frame.
    let sending=Promise.resolve();
    const send=x=>{
      const sealed=session.seal(JSON.stringify(x));
      sending=sending.then(async()=>socket.send(await sealed));
    };
    const reply=(id,result)=>send({jsonrpc:'2.0',id,result});
    const fail=(id,code,message)=>send({jsonrpc:'2.0',id,error:{code,message}});
    const update=(sessionId,change)=>send({
      jsonrpc:'2.0',method:'session/update',params:{sessionId,update:change},
    });
    async function processMessage(value,binary) {
      const handshake=value.toString();
      if (state==='hello') {
        auth=await createResponder(identity,{
          allowClient: async peerId=>peerId===allowedPeerId,
        });
        const challenge=await auth.challenge(handshake);
        state='proof';
        socket.send(challenge);
        return;
      }
      if (state==='proof') {
        const accepted=await auth.accept(handshake);
        session=accepted.session;
        socket.send(accepted.reply);
        state='ready';
        return;
      }
      if (!binary) throw Error('After the proof every message is a sealed record');
      const opened=await session.open(new Uint8Array(value));
      if (opened===null) {
        sending=sending.then(async()=>{socket.send(await session.close());socket.close();});
        return;
      }
      const rpc=JSON.parse(new TextDecoder().decode(opened));
      if (!rpc || rpc.jsonrpc!=='2.0' || typeof rpc.method!=='string') throw Error('Bad ACP RPC');
      const {id,method,params={}}=rpc;
      if (method==='initialize' && !initialized) {
        if (params.protocolVersion!==2) fail(id,-32602,'ACP v2 required');
        else {
          initialized=true;
          reply(id,{protocolVersion:2,capabilities:{session:{}},
            info:{name:'canoa-acp-echo',title:'Canoa Echo Fixture',version:'0.0.0'}});
        }
      } else if (!initialized) {
        if(id!=null)fail(id,-32000,'Initialize first');
      } else if(method==='session/new') {
        const sid=randomUUID(); sessions.set(sid,[]);
        reply(id,{sessionId:sid});
      } else if(method==='session/list') {
        reply(id,{sessions:[...sessions.keys()].map(sessionId=>({sessionId}))});
      } else if(method==='session/resume') {
        if(!sessions.has(params.sessionId)) fail(id,-32602,'Unknown session');
        else reply(id,{});
      } else if(method==='session/close') {
        if(!sessions.delete(params.sessionId)) fail(id,-32602,'Unknown session');
        else reply(id,{});
      } else if(method==='session/cancel') {
        if(sessions.has(params.sessionId)) update(params.sessionId,
          {sessionUpdate:'state_update',state:'idle',stopReason:'cancelled'});
      } else if(method==='session/prompt') {
        const messages=sessions.get(params.sessionId);
        if(!messages || !Array.isArray(params.prompt) || !params.prompt.length ||
          params.prompt.some(x=>x?.type!=='text' || typeof x.text!=='string')) {
          fail(id,-32602,'Invalid text prompt or session');
          return;
        }
        const text=params.prompt.map(x=>x.text).join('\n');
        const messageId=randomUUID(),agentId=randomUUID();
        messages.push({messageId,text});
        reply(id,{messageId});
        update(params.sessionId,{sessionUpdate:'user_message',messageId,
          content:[{type:'text',text}]});
        update(params.sessionId,{sessionUpdate:'state_update',state:'running'});
        update(params.sessionId,{sessionUpdate:'agent_message_chunk',messageId:agentId,
          content:{type:'text',text:'Echo: '+text}});
        update(params.sessionId,{sessionUpdate:'state_update',state:'idle',stopReason:'end_turn'});
      } else if(id!=null) {
        fail(id,-32601,'Not supported by echo fixture');
      }
    }
    socket.on('message',(data,binary)=>{
      chain=chain.then(()=>processMessage(data,binary))
        .catch(()=>socket.close(1008,'Peer proof or protocol refused'));
    });
  });
  await new Promise((resolve,reject)=>{
    httpsServer.once('error',reject);
    httpsServer.listen(port,host,resolve);
  });
  return {
    port:httpsServer.address().port,host,peerId:identity.peerId,
    async close(){
      for(const socket of sockets)socket.terminate();
      await new Promise(resolve=>wss.close(resolve));
      await new Promise(resolve=>httpsServer.close(resolve));
    },
  };
}
if(process.argv[1] && import.meta.url.endsWith('/'+process.argv[1].split('/').at(-1))){
  if(!process.env.CANOA_CERT || !process.env.CANOA_KEY ||
    !process.env.CANOA_ALLOWED_PEER_ID){
    throw Error('Set CANOA_CERT, CANOA_KEY and CANOA_ALLOWED_PEER_ID');
  }
  const identity=await loadIdentity(process.env.CANOA_IDENTITY_KEY);
  const service=await serve({
    cert:readFileSync(process.env.CANOA_CERT),key:readFileSync(process.env.CANOA_KEY),
    identity,allowedPeerId:process.env.CANOA_ALLOWED_PEER_ID,
    host:process.env.CANOA_HOST || '127.0.0.1',
    port:Number(process.env.CANOA_PORT || 8765),
  });
  console.log('WSS test fixture: wss://'+service.host+':'+service.port);
  console.log('Pinned server peer: '+service.peerId);
  if(!process.env.CANOA_IDENTITY_KEY)console.warn('Identity is ephemeral for this run');
}
