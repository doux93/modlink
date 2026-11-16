import { chromium } from 'playwright-core';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
// Fixture-only UI check: no physical ports, AI calls, or native data writes.
assert.equal(process.platform, 'win32');
assert.equal(process.env.GITHUB_ACTIONS, 'true');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const output = path.join(root, 'artifacts/desktop-smoke/product-fixtures');
const dist = path.join(root, 'dist');
const mime = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml' };
const server = createServer(async (req, res) => {
 try {
  const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
  const file = path.resolve(dist, '.' + (pathname === '/' ? '/index.html' : pathname));
  if (!file.startsWith(dist + path.sep)) { res.writeHead(403).end(); return; }
  const body = await readFile(file);
  res.writeHead(200, { 'Content-Type': mime[path.extname(file)] || 'application/octet-stream' }).end(body);
 } catch { res.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const url = `http://127.0.0.1:${server.address().port}`;

await mkdir(output,{recursive:true});
let browser;
try { browser = await chromium.launch({channel:'msedge', headless:true}); } catch (error) { server.close(); throw error; }
const page=await browser.newPage({viewport:{width:1280,height:850}});
const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.addInitScript(()=>{
 localStorage.setItem('modulink-onboarding-completed','1');localStorage.setItem('ml.theme','light');
 const callbacks=new Map();let next=1;
 const flow={id:'network_attach',name:'入网连通性检查',kind:'checkup',verified:true,category:'network',execution_mode:'readonly',applies_to:[],source:'内置基础检查',steps:[{id:'sim',title:'SIM 卡',command:'AT+CPIN?',timeout_ms:2000}]};
 const knowledge={profiles:[{id:'generic',schema:'',name:'通用蜂窝模组',support_level:'generic',at_commands:[],intents:[],capabilities:[],ports:[],urc_patterns:[],model_codes:[],diagnostics:[],error_code_tables:[],family:'',series:'',vendor:'',line_ending:'\r\n'}],flows:[flow],error_codes:{},profile_error_codes:{},at_docs:[],warnings:[]};
 window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
 window.__TAURI_INTERNALS__={transformCallback:(callback)=>{const id=next++;callbacks.set(id,callback);return id;},unregisterCallback:id=>callbacks.delete(id),invoke:async(command,args)=>{
  if(command==='plugin:event|listen')return next++;
  if(command==='plugin:event|unlisten')return;
  if(command==='get_knowledge')return knowledge;
  if(command==='list_serial_ports')return [{name:'COM3',description:'USB Serial Device'},{name:'COM7',description:'Diagnostic Log Port'}];
  if(command==='open_serial_ports'||command==='list_executions')return [];
  if(command==='load_quickcmds')return null;
  if(command==='save_quickcmds')return;
  if(command==='list_applicable_flows')return ['network_attach'];
  if(command==='get_knowledge_capabilities')return {coreAvailable:true,manualMode:'preview',effectiveManualMode:'preview',installedPacks:[],modelCoverage:[],fulltextModelCoverage:[],topicCount:0,degradationReason:'仅有摘要参考'};
  if(command==='get_settings')return {ai:{base_url:'',api_key_configured:false},kb:{manuals_dir:''}};
  if(command==='open_serial')return {port:args.port,baud:args.baud,generation:1};
  if(command==='run_diagnostic')return {flow_id:'network_attach',flow_name:flow.name,overall:'fail',steps:[{id:'sim',title:'SIM 卡检查',result:'fail',detail:'SIM 卡未就绪',reason:'设备返回 +CME ERROR: 10，未检测到 SIM 卡。',suggestion:'检查卡槽接触和卡片方向，重新上电后复测。',raw:['AT+CPIN?','+CME ERROR: 10'],collected:[],elapsed_ms:340}]};
  if(command==='get_capability_evidence_context')return '';
  throw new Error('Unexpected preview IPC: '+command);
 }};
});
try{
 await page.goto(url);await page.locator('.console').waitFor();
 await page.screenshot({path:output+'/light-console.png'});
 await page.locator('.device-context').getByRole('button',{name:'连接设备',exact:true}).click();
 await page.locator('.connection-dialog').waitFor();
 await page.getByLabel('波特率', {exact:true}).fill('57600');
 await page.screenshot({path:output+'/light-connection.png'});
 await page.getByRole('button',{name:'连接并识别',exact:true}).click();
 await page.locator('.connection-dialog').waitFor({state:'hidden'});
 assert((await page.locator('.device-context').innerText()).includes('57600'));
 await page.getByRole('link',{name:'设备诊断',exact:true}).click();await page.locator('.diag').waitFor();
 await page.screenshot({path:output+'/light-diagnosis-ready.png'});
 await page.getByRole('button',{name:'开始检查',exact:true}).click();
 await page.locator('.result-review').waitFor();
 await page.screenshot({path:output+'/light-diagnosis-result.png'});
 await page.getByRole('button',{name:'AI 分析异常',exact:true}).click();await page.locator('.ai').waitFor();
 assert((await page.locator('.ai textarea').first().inputValue()).includes('+CME ERROR: 10'));
 await page.screenshot({path:output+'/light-ai.png'});
 await page.locator('.sidebar-footer [role=switch]').click();
 await page.screenshot({path:output+'/dark-ai.png'});
 await page.getByRole('link',{name:'串口控制台',exact:true}).click();await page.locator('.console').waitFor();
 await page.setViewportSize({width:1028,height:749});
 await page.screenshot({path:output+'/dark-console-compact.png'});
 assert(await page.evaluate(() => document.querySelector('.shell').scrollWidth <= window.innerWidth), 'Compact shell must not overflow horizontally');
 assert.deepEqual(errors,[]);
 await writeFile(path.join(output,'result.json'), JSON.stringify({passed:true, fixture:true, commit:process.env.GITHUB_SHA, checks:['connection with custom settings', 'diagnostic result', 'AI draft handoff without sending', 'compact layout'], screenshots:7},null,2));
 console.log('Visual preview flows passed: connected fixture, diagnosis, AI handoff, 7 screenshots; no hardware used.');
}catch(error){
 await page.screenshot({path:path.join(output,'failure.png')}).catch(()=>{});
 await writeFile(path.join(output,'result.json'), JSON.stringify({passed:false, fixture:true, errors, error:String(error)},null,2));
 throw error;
}finally{await browser.close();server.close();}
