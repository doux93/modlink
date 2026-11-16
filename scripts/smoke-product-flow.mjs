import { chromium } from 'playwright-core';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { mkdir, readdir, writeFile } from 'node:fs/promises';
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
async function screenshot(name) {
 await page.locator('.n-message').last().waitFor({state:'hidden'});
 await page.evaluate(() => document.fonts.ready);
 await page.screenshot({path:path.join(output,name),animations:'disabled'});
}
await page.addInitScript(()=>{
 localStorage.setItem('modulink-onboarding-completed','1');localStorage.setItem('ml.theme','light');
 const callbacks=new Map();const listeners=new Map();let next=1;let failSettings=true;let failLogClose=true;let failAi=true;
 window.__emitFixtureEvent=(name,payload)=>callbacks.get(listeners.get(name))?.({event:name,payload,id:0});
 const flow={id:'network_attach',name:'入网连通性检查',kind:'checkup',verified:true,category:'network',execution_mode:'readonly',applies_to:[],source:'内置基础检查',steps:[{id:'sim',title:'SIM 卡',command:'AT+CPIN?',timeout_ms:2000}]};
 const knowledge={profiles:[{id:'generic',schema:'',name:'通用蜂窝模组',support_level:'generic',at_commands:[],intents:[],capabilities:[],ports:[],urc_patterns:[],model_codes:[],diagnostics:[],error_code_tables:[],family:'',series:'',vendor:'',line_ending:'\r\n'}],flows:[flow],error_codes:{},profile_error_codes:{},at_docs:[],warnings:[]};
 window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
 window.__TAURI_INTERNALS__={transformCallback:(callback)=>{const id=next++;callbacks.set(id,callback);return id;},unregisterCallback:id=>callbacks.delete(id),invoke:async(command,args)=>{
  if(command==='plugin:event|listen'){listeners.set(args.event,args.handler);return next++;}
  if(command==='plugin:event|unlisten')return;
  if(command==='get_knowledge')return knowledge;
  if(command==='list_serial_ports')return [{name:'COM3',description:'USB Serial Device'},{name:'COM7',description:'Second AT Port'},{name:'COM8',description:'Diagnostic Log Port'}];
  if(command==='open_serial_ports'||command==='list_executions')return [];
  if(command==='load_quickcmds')return JSON.parse(localStorage.getItem('fixture.quickCommands') || 'null');
  if(command==='save_quickcmds'){if(window.__failNextQuickSave){window.__failNextQuickSave=false;throw new Error('Fixture: save denied');}localStorage.setItem('fixture.quickCommands',JSON.stringify(args.data));return;}
  if(command==='list_applicable_flows')return ['network_attach'];
  if(command==='get_knowledge_capabilities')return {coreAvailable:true,manualMode:'preview',effectiveManualMode:'preview',installedPacks:[],modelCoverage:[],fulltextModelCoverage:[],topicCount:0,degradationReason:'仅有摘要参考'};
  if(command==='get_settings')return {ai:{base_url:'',api_key_configured:false},kb:{manuals_dir:''}};
  if(command==='save_settings'){if(failSettings){failSettings=false;throw new Error('Fixture: configuration write denied');}return;}
  if(command==='prepare_ai_request'||command==='cancel_ai_request')return true;
  if(command==='ai_chat'){if(failAi){failAi=false;throw new Error('Fixture: service unavailable');}return {choices:[{message:{content:'OK'}}]};}
  if(command==='close_serial'){if(args.port==='COM8' && failLogClose){failLogClose=false;throw new Error('Fixture: log port close denied');}return;}
  if(command==='open_serial')return {port:args.port,baud:args.baud,generation:1};
  if(command==='run_diagnostic')return {flow_id:'network_attach',flow_name:flow.name,overall:'fail',steps:[{id:'sim',title:'SIM 卡检查',result:'fail',detail:'SIM 卡未就绪',reason:'设备返回 +CME ERROR: 10，未检测到 SIM 卡。',suggestion:'检查卡槽接触和卡片方向，重新上电后复测。',raw:['AT+CPIN?','+CME ERROR: 10'],collected:[],elapsed_ms:340}]};
  if(command==='get_capability_evidence_context')return '';
  throw new Error('Unexpected preview IPC: '+command);
 }};
});
try{
 await page.goto(url);await page.locator('.console').waitFor();
 await page.locator('.quick').waitFor({state:'hidden'});
 await screenshot('light-console.png');
 // Restore saved commands with no explicit panel choice: wide screens open automatically.
 await page.getByRole('button',{name:'展开快捷指令',exact:true}).click();
 await page.getByRole('button',{name:'或手动新建一条',exact:true}).click();
 await page.getByPlaceholder('AT 指令或数据',{exact:true}).fill('AT+CSQ');
 await page.locator('.n-modal').getByRole('button',{name:'保存',exact:true}).click();
 await page.waitForFunction(()=>localStorage.getItem('fixture.quickCommands')?.includes('AT+CSQ'));
 await page.evaluate(()=>localStorage.removeItem('ml.showQuickCommands'));
 await page.reload();
 await page.locator('.quick-text').filter({hasText:'AT+CSQ'}).waitFor();

 await page.locator('.device-context').getByRole('button',{name:'连接设备',exact:true}).click();
 await page.locator('.connection-dialog').waitFor();
 await page.getByLabel('波特率', {exact:true}).fill('57600');
 await screenshot('light-connection.png');
 await page.getByRole('button',{name:'连接并识别',exact:true}).click();
 await page.locator('.connection-dialog').waitFor({state:'hidden'});
 assert((await page.locator('.device-context').innerText()).includes('57600'));
 assert((await page.locator('.connection-state').innerText()).includes('串口已连接'));
 await page.locator('.model-label').filter({hasText:'型号待识别'}).waitFor();
 await page.locator('.device-details summary').click();
 assert((await page.locator('.device-availability').innerText()).includes('通用基础诊断'));
 await screenshot('light-device-status.png');
 await page.locator('.device-details summary').click();
 await page.getByRole('link',{name:'设备诊断',exact:true}).click();await page.locator('.diag').waitFor();
 await screenshot('light-diagnosis-ready.png');
 await page.getByRole('button',{name:'开始检查',exact:true}).click();
 await page.locator('.result-review').waitFor();
 await screenshot('light-diagnosis-result.png');
 await page.getByRole('button',{name:'AI 分析异常',exact:true}).click();await page.locator('.ai').waitFor();
 assert((await page.locator('.ai textarea').first().inputValue()).includes('+CME ERROR: 10'));
 await screenshot('light-ai.png');
 await page.locator('.ai-head').getByRole('button',{name:'配置',exact:true}).click();
 await page.getByPlaceholder('Base URL，如 https://onerouter.cmaiot.cn/v1',{exact:true}).fill('https://fixture.invalid/v1');
 await page.getByPlaceholder('API Key',{exact:true}).fill('fixture-key');
 await page.locator('.cfg-modal').getByRole('button',{name:'保存',exact:true}).click();
 await page.locator('.cfg-error').waitFor();
 const settingsDialog = await page.locator('.cfg-modal').boundingBox();
 assert(settingsDialog.width <= 521 && settingsDialog.x >= 16, 'Settings dialog must remain compact and inside the viewport');
 assert((await page.getByPlaceholder('Base URL，如 https://onerouter.cmaiot.cn/v1',{exact:true}).inputValue())==='https://fixture.invalid/v1');
 assert((await page.locator('.ai-head').innerText()).includes('AI 待配置'));
 await screenshot('light-ai-settings-retry.png');
 await page.locator('.cfg-modal').getByRole('button',{name:'保存',exact:true}).click();
 await page.locator('.cfg-modal').waitFor({state:'hidden'});
 assert(!(await page.locator('.ai-head').innerText()).includes('AI 待配置'));
 assert((await page.locator('.ai-head').innerText()).includes('AI 尚未验证'));
 await page.locator('.ai-head').getByRole('button',{name:'测试连接',exact:true}).click();
 await page.locator('.connection-error').waitFor();
 assert((await page.locator('.ai-head').innerText()).includes('AI 连接失败'));
 await screenshot('light-ai-connection-failed.png');
 await page.locator('.ai-head').getByRole('button',{name:'测试连接',exact:true}).click();
 await page.waitForFunction(()=>document.querySelector('.ai-head').textContent.includes('AI 可用'));

 await page.locator('.sidebar-footer [role=switch]').click();
 await screenshot('dark-ai.png');
 await page.getByRole('link',{name:'串口控制台',exact:true}).click();await page.locator('.console').waitFor();
 if(!await page.locator('.quick').isVisible())await page.getByRole('button',{name:'展开快捷指令',exact:true}).click();
 await page.evaluate(()=>{window.__failNextQuickSave=true;});
 await page.locator('.quick-item').first().hover();
 await page.getByRole('button',{name:'编辑指令',exact:true}).first().click();
 await page.getByPlaceholder('AT 指令或数据',{exact:true}).fill('AT+CGMI');
 await page.locator('.n-modal').getByRole('button',{name:'保存',exact:true}).click();
 await page.locator('.quick-save-alert').waitFor();
 assert((await page.locator('.quick-text').first().innerText()).includes('AT+CGMI'));
 await screenshot('dark-quick-save-failed.png');
 await page.locator('.quick-save-alert').getByRole('button',{name:'重试保存',exact:true}).click();
 await page.locator('.quick-save-alert').waitFor({state:'hidden'});

 await page.setViewportSize({width:1028,height:749});
 await page.evaluate(() => window.__emitFixtureEvent('serial://frames', [
  {generation:1,seq:1,ts:Date.now(),port:'COM3',dir:'tx',kind:'tx',text:'AT+CSQ'},
  {generation:1,seq:2,ts:Date.now(),port:'COM3',dir:'rx',kind:'response',text:'+CSQ: 23,99'},
  {generation:1,seq:3,ts:Date.now(),port:'COM3',dir:'rx',kind:'final',text:'OK'},
  {generation:1,seq:4,ts:Date.now(),port:'COM3',dir:'rx',kind:'final',text:'+CME ERROR: 10'},
 ]));
 await page.locator('.terminal .line').first().waitFor();
 await screenshot('dark-console-compact.png');
 const compact = await page.evaluate(() => ({
  headingHeight:document.querySelector('.workspace-heading').getBoundingClientRect().height,
  terminalHeight:document.querySelector('.io .terminal').getBoundingClientRect().height,
  terminalWidth:document.querySelector('.io .terminal').getBoundingClientRect().width,
  redundantTabs:document.querySelectorAll('.session-tabs').length,
 }));
 assert(compact.headingHeight <= 44, 'Console heading must remain one compact row');
 assert(compact.terminalHeight >= 420, 'Compact window must prioritize log height');
 assert.equal(compact.redundantTabs,0, 'A single device must not repeat the device selector in a tab row');
 // Exercise the real connection UI with a second fixture device and a paired log port.
 await page.locator('.device-context').getByRole('button',{name:'添加设备',exact:true}).click();
 await page.locator('.connection-dialog').waitFor();
 await page.getByText('高级连接：同时采集日志串口',{exact:true}).click();
 await page.locator('.connection-dialog .n-select').nth(1).click();
 await page.getByText('COM8 · Diagnostic Log Port',{exact:true}).click();
 await page.getByRole('button',{name:'连接并识别',exact:true}).click();
 await page.locator('.connection-dialog').waitFor({state:'hidden'});
 await page.locator('.logpane').waitFor();
 await page.evaluate(() => {
  window.__emitFixtureEvent('serial://frames', [{generation:1,seq:1,ts:Date.now(),port:'COM7',dir:'rx',kind:'response',text:'+CREG: 0,1; '+ 'registration_details_'.repeat(12)}, {generation:1,seq:1,ts:Date.now(),port:'COM8',dir:'rx',kind:'raw',text:'[NETWORK] Registration accepted; data connection ready.'}]);
 });
 await page.locator('.quick').waitFor({state:'hidden'});
 await screenshot('dark-console-dual-port.png');
 const dual = await page.evaluate(() => ({
  terminalWidth:document.querySelector('.io').getBoundingClientRect().width,
  logWidth:document.querySelector('.logpane').getBoundingClientRect().width,
  quickHidden:getComputedStyle(document.querySelector('.quick')).display==='none',
  terminalOverflow:document.querySelector('.io .terminal').scrollWidth-document.querySelector('.io .terminal').clientWidth,
  logToolbarLabelHeight:Math.max(...[...document.querySelectorAll('.logpane-head b, .logpane-ops .n-checkbox__label')].map(el=>el.getBoundingClientRect().height)),
 }));
 assert(dual.terminalWidth >= 440 && dual.logWidth >= 260 && dual.quickHidden, 'Narrow dual-port workspaces prioritize AT and log columns');
 assert(dual.terminalOverflow <= 1, 'Log text must wrap inside narrow columns');
 assert(dual.logToolbarLabelHeight <= 24, 'Log toolbar labels must not break across lines in narrow columns');
 // Check actual composited text/button colors in the rendered dark theme.
 const contrast = await page.evaluate(() => {
  const rgb = color => color.match(/[\d.]+/g).slice(0,3).map(Number);
  const luminance = color => rgb(color).map(v=>v/255).map(v=>v<=0.04045?v/12.92:((v+0.055)/1.055)**2.4).reduce((sum,v,i)=>sum+v*[0.2126,0.7152,0.0722][i],0);
  const ratio = (a,b) => (Math.max(luminance(a),luminance(b))+0.05)/(Math.min(luminance(a),luminance(b))+0.05);
  const terminal=getComputedStyle(document.querySelector('.io .terminal'));
  const timestamp=getComputedStyle(document.querySelector('.io .ts'));
  const button=[...document.querySelectorAll('.input-side button')].find(el=>el.textContent.trim()==='发送');
  const buttonStyle=getComputedStyle(button);
  return {timestamp:ratio(timestamp.color,terminal.backgroundColor),primaryButton:ratio(buttonStyle.color,buttonStyle.backgroundColor)};
 });
 assert(contrast.timestamp >= 4.5 && contrast.primaryButton >= 4.5, 'Dark text must stay readable against its actual background');

 assert(await page.evaluate(() => document.querySelector('.shell').scrollWidth <= window.innerWidth), 'Compact shell must not overflow horizontally');
 await page.getByRole('button',{name:'展开快捷指令',exact:true}).click();
 await page.locator('.quick-floating').waitFor();
 assert(Math.abs((await page.locator('.io').boundingBox()).width-dual.terminalWidth)<1, 'Opening quick commands must not squeeze the main terminal');
 await screenshot('dark-console-quick-floating.png');
 await page.getByRole('button',{name:'关闭快捷指令面板',exact:true}).focus();
 await page.keyboard.press('Escape');
 await page.locator('.quick').waitFor({state:'hidden'});
 assert(await page.getByRole('button',{name:'展开快捷指令',exact:true}).evaluate(el=>el===document.activeElement));
 await page.getByRole('button',{name:'展开快捷指令',exact:true}).click();
 await page.setViewportSize({width:1600,height:900});
 await page.waitForFunction(()=>!document.querySelector('.quick').classList.contains('quick-floating'));
 assert((await page.locator('.io').boundingBox()).width>=440);
 await screenshot('dark-console-wide.png');
 await page.getByRole('button',{name:'收起快捷指令',exact:true}).click();
 await page.setViewportSize({width:1028,height:749});
 await page.locator('.quick').waitFor({state:'hidden'});
 assert(await page.evaluate(()=>localStorage.getItem('ml.showQuickCommands')==='0'), 'Resize must preserve an explicit closed preference');
 await page.getByRole('button',{name:'关闭设备 COM7',exact:true}).click();
 await page.locator('.connection-state').filter({hasText:'串口已断开'}).waitFor();
 await page.getByRole('button',{name:'关闭设备 COM7',exact:true}).waitFor();
 assert(await page.locator('.logpane').isVisible(), 'A failed log close must keep the group available for retry');
 await screenshot('dark-partial-close.png');
 await page.getByRole('button',{name:'关闭设备 COM7',exact:true}).click();
 await page.locator('.logpane').waitFor({state:'hidden'});
 assert((await page.locator('.device-context').innerText()).includes('COM3'));


 assert.deepEqual(errors,[]);
 const screenshots=(await readdir(output)).filter(name=>name.endsWith('.png')).length;
 await writeFile(path.join(output,'result.json'), JSON.stringify({passed:true, fixture:true, commit:process.env.GITHUB_SHA, checks:['connection with custom settings', 'diagnostic result', 'AI draft handoff without sending', 'compact layout', 'compact heading and log height', 'single-device tab removal', 'empty panel stays closed', 'saved commands open automatically', 'dual-port automatic collapse', 'floating panel preserves terminal width', 'panel Escape returns focus', 'wide inline layout', 'explicit closed preference survives resize', 'device state and available actions', 'dark text contrast', 'settings failure preserves draft and saved state', 'settings retry applies saved state', 'partial group close retains state and retries', 'quick save failure retains edits and retries', 'AI is unverified until a successful test', 'AI failed connection retries without sending device logs'], screenshots, compact, dual, contrast},null,2));
 console.log(`Visual preview flows passed: connected fixture, diagnosis, AI handoff, ${screenshots} screenshots; no hardware used.`);
}catch(error){
 await page.screenshot({path:path.join(output,'failure.png')}).catch(()=>{});
 await writeFile(path.join(output,'result.json'), JSON.stringify({passed:false, fixture:true, errors, error:String(error)},null,2));
 throw error;
}finally{await browser.close();server.close();}
