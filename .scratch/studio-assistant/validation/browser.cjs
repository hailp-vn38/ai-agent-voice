const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({ headless: true, args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, permissions: ['microphone'] });
  await context.addInitScript(() => {
    sessionStorage.setItem('voice-agent-admin-token','test-admin');
    localStorage.setItem('voice-agent-admin-locale','en');
    localStorage.setItem('voice-agent-admin-theme','light');
  });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  let settings = { configured: true, enabled: true, device_id: 'web_assistant_test', agent_key: 'studio', template_key: null, revision: 1, active_connections: 0 };
  let tickets = 0, sockets = 0, detections = 0;
  await page.route('**/api/admin/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    let value = { items: [], page: 1, page_size: 50 };
    if (path.endsWith('/assistant-device')) {
      if(request.method() === 'PUT') settings = { ...settings, ...request.postDataJSON(), revision: settings.revision+1 };
      value = settings;
    } else if(path.endsWith('/ws-ticket')) value = { device_id: settings.device_id, client_id: `web_${++tickets}`, ticket: `ticket_${tickets}`, expires_in_seconds: 30 };
    else if(path.endsWith('/agents')) value.items = [{ key: 'studio', name: 'Studio', enabled: true, revision: 1 }];
    else if(path.endsWith('/agents/studio/templates')) value.items = [{ key: 'assistant_vi', name: 'Assistant VI', language: 'vi', is_default: true, enabled: true }];
    else if(path.endsWith('/templates')) value.items = [{ key: 'assistant_vi', name: 'Assistant VI', language: 'vi', prompt: 'help', enabled: true, revision: 1 }];
    else if(path.endsWith('/templates/assistant_vi/providers')) value = { template_key: 'assistant_vi', revision: 1, bindings: {} };
    else if(path.endsWith('/system')) value = { status:'ready',version:'test',uptime_seconds:10,database:{enabled:true,status:'ready'},providers:{configured:4,loaded:4,stale:0,failed:0},sessions:{active:0} };
    await route.fulfill({ json: value });
  });
  await page.route('**/health', route => route.fulfill({ body:'ok' }));
  await page.route('**/ready', route => route.fulfill({ body:'ready' }));
  await page.routeWebSocket(/\/voice\/v1\//, ws => {
    ++sockets;
    const session_id = `session_${sockets}`;
    const send = event => ws.send(JSON.stringify({ session_id, ...event }));
    let ready = false;
    ws.onMessage(raw => {
      if(typeof raw !== 'string') return;
      const message = JSON.parse(raw);
      if(message.type === 'hello') {
        assert.equal(message.features.turn_status,true); assert.equal(message.features.mcp,false);
        setTimeout(() => send({type:'hello',transport:'websocket',audio_params:{format:'opus',sample_rate:24000,channels:1,frame_duration:60}}),100);
      } else if(message.state === 'start') {
        setTimeout(() => { ready=true; send({type:'turn_status',state:'listening_ready'}); }, 100);
      } else if(message.state === 'detect') {
        assert.equal(ready,true); ready=false; ++detections;
        send({type:'stt',text:message.text});
        send({type:'llm',text:'Xin chào. '});send({type:'llm',text:'Tôi có thể hỗ trợ bạn.'});
        send({type:'tts',state:'start'}); ws.send(Buffer.from([24,0])); send({type:'tts',state:'stop'});
        send({type:'turn_status',state:'completed',turn_id:detections});
      } else if(message.type==='abort') send({type:'turn_status',state:'cancelled'});
    });
  });
  await page.goto('http://127.0.0.1:5198/system');

  await page.getByText('Studio Assistant',{exact:true}).first().waitFor();
  await page.getByRole('button',{name:'Open Floating Chat',exact:true}).first().click();
  const panel = page.locator('#studio-assistant');
  await panel.getByText('Connected',{exact:true}).waitFor();
  await panel.getByRole('textbox').fill('Xin chào');
  await panel.getByRole('button',{name:'Send',exact:true}).click();
  await panel.getByText('Xin chào. Tôi có thể hỗ trợ bạn.',{exact:true}).waitFor();
  assert.equal(await panel.locator('article').count(),2);
  await page.screenshot({ path: '.scratch/studio-assistant/validation/desktop-light.png' });
  await panel.getByRole('button',{name:'Collapse chat'}).click();
  await page.locator('a[href="/agents"]').first().click();
  await page.getByRole('button',{name:'Open Floating Chat',exact:true}).last().click();
  await panel.getByText('Xin chào. Tôi có thể hỗ trợ bạn.',{exact:true}).waitFor();
  assert.equal(sockets,1); assert.equal(tickets,1);
  await page.evaluate(() => { document.documentElement.classList.add('dark') });
  await page.screenshot({path:'.scratch/studio-assistant/validation/desktop-dark.png'});
  await page.setViewportSize({width:390,height:844});
  await page.screenshot({path:'.scratch/studio-assistant/validation/mobile.png'});
  await panel.getByRole('button',{name:'Interrupt',exact:true}).focus().catch(()=>{});
  await page.keyboard.press('Escape');
  await panel.waitFor({state:'hidden'});
  // Verify native microphone encoding + raw Opus decoding independently of the mocked WS.
  const audio = await page.evaluate(async () => {
    const { AssistantAudio, isCanonicalOpusPacket } = await import('/src/lib/assistant/audio.ts');
    const errors=[];let packets=0;let first;
    const audio = new AssistantAudio(code=>errors.push(code));
    await audio.enableSpeaker();
    await audio.startMicrophone(packet => { packets++;first??=packet;if(!isCanonicalOpusPacket(packet)) errors.push('bad_packet'); });
    await new Promise(resolve=>setTimeout(resolve,500));
    await audio.stopMicrophone();
    if(first){ audio.beginPlayback();audio.receive(first);await new Promise(resolve=>setTimeout(resolve,200)); }
    audio.disconnect();
    return {packets,errors};
  });
  assert.ok(audio.packets>0); assert.deepEqual(audio.errors,[]); assert.deepEqual(errors,[]);
  console.log(JSON.stringify({ desktop:'PASS', mobile:'PASS', routePersistence:'PASS', textAck:'PASS', nativeOpus:audio, tickets,sockets }));
  await browser.close();
})().catch(e => { console.error(e); process.exit(1) });
