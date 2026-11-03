const form=document.querySelector('#grant-form');
const tabSelect=document.querySelector('#tab');
const list=document.querySelector('#grants');
const status=document.querySelector('#status');
const tabOrigins=new Map();
const options={peer:document.querySelector('#peer'),
  capability:document.querySelector('#capability'),
  minutes:document.querySelector('#minutes')};

async function request(message){
  const reply=await chrome.runtime.sendMessage(message);
  if (reply?.error) throw Error(reply.error);
  return reply;
}
function show(error){status.textContent=String(error);}
async function tabs(){
  const all=await chrome.tabs.query({});
  tabSelect.replaceChildren();
  tabOrigins.clear();
  for (const tab of all){
    if (!Number.isSafeInteger(tab.id) || !/^https?:\/\//.test(tab.url||'')) continue;
    const item=document.createElement('option');
    tabOrigins.set(tab.id,new URL(tab.url).origin);
    item.value=String(tab.id);
    item.textContent=(tab.title||'Website').slice(0,80)+' — '+new URL(tab.url).origin;
    tabSelect.append(item);
  }
}
async function refresh(){
  const grants=(await request({type:'canoa.grants.list'})).grants;
  list.replaceChildren();
  for (let i=0;i<grants.length;i++){
    const grant=grants[i];
    const row=document.createElement('li');
    const text=document.createElement('code');
    text.textContent=grant.capability+' • tab '+grant.tabId+' • '+grant.origin+
      ' • '+grant.peerId.slice(0,24)+'… • expires '+new Date(grant.expiresAt).toLocaleString();
    const button=document.createElement('button');
    button.textContent='Revoke';
    button.addEventListener('click',async()=>{
      try{await request({type:'canoa.grants.remove',grant});await refresh();}
      catch(e){show(e.message);}
    });
    row.append(text,' ',button);
    list.append(row);
  }
}
form.addEventListener('submit',async event=>{
  event.preventDefault();
  try{
    const tabId=Number(tabSelect.value);
    const origin=tabOrigins.get(tabId);
    if (!origin) throw Error('Select a website tab first');
    // Ask Chrome for real host permission during this user gesture. A
    // Canoa policy grant alone cannot bypass the browser permission gate.
    const allowed=await chrome.permissions.request({origins:[origin+'/*']});
    if (!allowed) throw Error('Browser host access was refused');
    await request({
      type:'canoa.grants.add',
      peerId:options.peer.value.trim(),
      tabId,
      capability:options.capability.value,
      minutes:Number(options.minutes.value),
    });
    show('Permission added. The verified agent still needs a valid Coqui connection.');
    await refresh();
  }catch(e){show(e.message);}
});
const machine=document.querySelector('#machine');
const standingToggle=document.querySelector('#standing-toggle');
const standingStatus=document.querySelector('#standing-status');
const standingCommand=document.querySelector('#standing-command');
const STANDING_SAYS={
  off:'Off.',
  connecting:'Connecting to that machine.',
  connected:'On. Programs on that machine can ask for a tab.',
  unreachable:'On, but that machine cannot be reached yet, or did not prove its key. This keeps trying.',
  refused:'On, but that machine refused: it needs coqui-acp-bridge serve --mcp-offers, allowing this browser.',
};
let standingOn=null;
async function refreshStanding(){
  const view=await request({type:'canoa.standing.get'});
  standingOn=view.on;
  const chosen=view.on ?? machine.value;
  machine.replaceChildren(...view.machines.map(({key,label,peer,serves})=>{
    const item=document.createElement('option');
    item.value=key;
    item.disabled=!serves;
    item.textContent=label.slice(0,80)+' — '+peer.slice(0,20)+'…'+(serves?'':' (does not say it can do this)');
    return item;
  }));
  if (view.machines.some(({key})=>key===chosen)) machine.value=chosen;
  machine.disabled=view.on!==null;
  standingToggle.textContent=view.on===null?'Turn on':'Turn off';
  standingToggle.disabled=view.on===null && !view.machines.some(({serves})=>serves);
  standingStatus.textContent=view.machines.length===0?'No paired machine yet: add an agent under Canoa agents first.':STANDING_SAYS[view.status.state]??'';
  // The key this browser proves by to the chosen machine, whose method decides which; never a key it does not have.
  const shown=view.machines.find(({key})=>key===machine.value);
  standingCommand.textContent=shown===undefined
    ?Object.entries(view.ids).filter(([,id])=>id!==null).map(([method,id])=>method+': '+id).join('\n')
    :shown.command??('This browser cannot prove by '+shown.method+' here, so it has no key for that machine.');
}
standingToggle.addEventListener('click',async()=>{
  try{
    await request({type:'canoa.standing.set',agent:standingOn===null?machine.value:null});
    await refreshStanding();
  }catch(e){standingStatus.textContent=e.message;}
});
// Grants change behind this page: a tab closes, a navigation ends one, an agent is given another.
chrome.storage.onChanged.addListener((_,area)=>{
  if (area==='session'){
    refresh().catch(error=>show(error.message));
    refreshStanding().catch(error=>{standingStatus.textContent=error.message;});
  }
});
await tabs().catch(error=>show(error.message));
await refresh().catch(error=>show(error.message));
await refreshStanding().catch(error=>{standingStatus.textContent=error.message;});
