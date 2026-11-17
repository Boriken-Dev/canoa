import test from 'node:test';
import assert from 'node:assert/strict';
import {handleMcpRequest,toolsForPeer} from './mcp.js';
import {DIRECTION,CAPABILITY,makeGrant} from './permissions.js';
import {debugCommand,debugDetach,debugEvents} from './devtools.js';

const trusted = new Set(['verified-peer']);
const now = 10000;
const granted = cap => makeGrant({
  direction:DIRECTION.AGENT_TO_PAGE,peerId:'verified-peer',
  tabId:42,origin:'https://example.org',capability:cap,expiresAt:now+5000,
});
const rpc=(id,method,params={})=>({jsonrpc:'2.0',id,method,params});

test('MCP never accepts peer ID claimed in a message',async()=>{
  await assert.rejects(
    handleMcpRequest({request:rpc(1,'tools/list'),peerId:'unverified',
      trustedPeers:trusted,grants:[granted(CAPABILITY.READ)],dispatch:()=>{}}),
    /authentication required/);
});
test('MCP initialization and tools/list expose only granted capabilities',async()=>{
  const opts={peerId:'verified-peer',trustedPeers:trusted,
    grants:[granted(CAPABILITY.READ)],now,dispatch:()=>{}};
  const initialized=await handleMcpRequest({...opts,
    request:rpc(1,'initialize',{protocolVersion:'2025-11-25'})});
  assert.equal(initialized.result.capabilities.tools.listChanged,false);
  const listed=await handleMcpRequest({...opts,request:rpc(2,'tools/list')});
  assert.ok(listed.result.tools.some(t=>t.name==='browser_read_page'));
  assert.ok(!listed.result.tools.some(t=>t.name==='browser_click'));
  assert.ok(!listed.result.tools.some(t=>t.name==='browser_debug_command'));
});
test('MCP tools/call is denied without tab capability and respects grants',async()=>{
  let calls=0;
  const opts={peerId:'verified-peer',trustedPeers:trusted,
    grants:[granted(CAPABILITY.READ)],now,
    dispatch: async()=>{calls++;return {url:'https://example.org'};}};
  const denied=await handleMcpRequest({...opts,
    request:rpc(3,'tools/call',{name:'browser_click',arguments:{tabId:42,selector:'a'}})});
  assert.equal(denied.result.isError,true);
  assert.equal(calls,0);
  const allowed=await handleMcpRequest({...opts,
    request:rpc(4,'tools/call',{name:'browser_read_page',arguments:{tabId:42}})});
  assert.equal(allowed.result.isError,false);
  assert.equal(calls,1);
});
test('debugging requires a separate elevated grant',()=>{
  const allowed=toolsForPeer({peerId:'verified-peer',trustedPeers:trusted,
    grants:[granted(CAPABILITY.DEBUG)],now});
  assert.ok(allowed.some(t=>t.name==='browser_debug_command'));
  assert.ok(!allowed.some(t=>t.name==='browser_read_page'));
});
test('CDP rejects unsupported commands and invalid evaluation',async()=>{
  const api={onDetach:{addListener(){}},async attach(){},async sendCommand(){return {}}};
  await assert.rejects(()=>debugCommand(
    {tabId:7,method:'Target.attachToTarget'},api),/unavailable/);
  await assert.rejects(()=>debugCommand(
    {tabId:7,method:'Runtime.evaluate',params:{}},api),/evaluation/);
});
test('CDP attaches once, passes approved commands and can detach',async()=>{
  const events=[];
  const api={
    onDetach:{addListener(){}},
    async attach(target){events.push(['attach',target.tabId]);},
    async sendCommand(target,method){events.push(['send',target.tabId,method]);return {result:'ok'};},
    async detach(target){events.push(['detach',target.tabId]);},
  };
  const op={tabId:99,method:'DOM.getDocument',params:{}};
  assert.deepEqual(await debugCommand(op,api),{result:'ok'});
  await debugCommand(op,api);
  await debugDetach({tabId:99},api);
  assert.deepEqual(events.map(e=>e[0]),['attach','send','send','detach']);
});

test('CDP events are bounded to the debug session and erased on detach',async()=>{
  let onEvent;
  let onDetach;
  const api={
    onDetach:{addListener(fn){onDetach=fn;}},
    onEvent:{addListener(fn){onEvent=fn;}},
    async attach(){},
    async sendCommand(){return {}},
    async detach(){},
  };
  await debugCommand({tabId:321,method:'Runtime.enable'},api);
  for(let i=0;i<140;i++){
    onEvent({tabId:321},'Runtime.consoleAPICalled',{type:'log',message:'line '+i});
  }
  assert.equal(debugEvents({tabId:321}).events.length,120);
  onEvent({tabId:111},'Runtime.consoleAPICalled',{type:'log',message:'secret'});
  assert.equal(debugEvents({tabId:111}).events.length,0);
  onDetach({tabId:321});
  assert.equal(debugEvents({tabId:321}).events.length,0);
});
