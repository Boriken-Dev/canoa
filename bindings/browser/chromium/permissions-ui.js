const form=document.querySelector('#grant-form');
const tabSelect=document.querySelector('#tab');
const list=document.querySelector('#grants');
const status=document.querySelector('#status');
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
  for (const tab of all){
    if (!Number.isSafeInteger(tab.id) || !/^https?:\/\//.test(tab.url||'')) continue;
    const item=document.createElement('option');
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
      try{await request({type:'canoa.grants.remove',index:i});await refresh();}
      catch(e){show(e.message);}
    });
    row.append(text,' ',button);
    list.append(row);
  }
}
form.addEventListener('submit',async event=>{
  event.preventDefault();
  try{
    await request({
      type:'canoa.grants.add',
      peerId:options.peer.value.trim(),
      tabId:Number(tabSelect.value),
      capability:options.capability.value,
      minutes:Number(options.minutes.value),
    });
    show('Permission added. The verified agent still needs a valid Coqui connection.');
    await refresh();
  }catch(e){show(e.message);}
});
await tabs().catch(error=>show(error.message));
await refresh().catch(error=>show(error.message));
