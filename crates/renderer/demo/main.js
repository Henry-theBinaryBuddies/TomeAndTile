// Demo host for the tt-renderer WASM interface. No build step, no npm.
//   ?user=<user_id>   dev sign-in as that member (default: the fixture GM)
//   ?ws=<url>         socket URL (default ws://localhost:8788/ws)
//   ?campaign=<id>    Hello.campaign_id (default empty: the server skips the check)
import init, { Renderer, encodeHello } from './pkg/tt_renderer.js';

const params = new URLSearchParams(location.search);
const user = params.get('user') || 'mock-gm';
const url = params.get('ws') || 'ws://localhost:8788/ws';
const campaign = params.get('campaign') || '';

const statusEl = document.getElementById('status');
const canvas = document.getElementById('map');
let connection = 'connecting';
let rejection = '';
let lastError = '';

function showStatus() {
  const lines = [`user: ${user}   socket: ${connection}`];
  if (rejection) lines.push(`last rejection: ${rejection}`);
  statusEl.textContent = lines.join('\n');
  if (lastError) {
    const span = document.createElement('span');
    span.className = 'bad';
    span.textContent = `\nerror: ${lastError}`;
    statusEl.appendChild(span);
  }
}

function fail(e) {
  lastError = String((e && e.message) || e);
  console.error(e);
  showStatus();
}

async function main() {
  await init();
  const renderer = await Renderer.create(canvas);

  const fit = () => {
    const r = canvas.getBoundingClientRect();
    renderer.resize(r.width, r.height, window.devicePixelRatio || 1);
  };
  new ResizeObserver(fit).observe(canvas);
  fit();

  let socket = null;
  renderer.setIntentHandler((bytes) => {
    if (socket && socket.readyState === WebSocket.OPEN) socket.send(bytes);
  });
  renderer.setRejectionHandler((code, message) => {
    rejection = `${code}: ${message}`;
    showStatus();
  });

  function connect() {
    connection = 'connecting';
    showStatus();
    socket = new WebSocket(url);
    socket.binaryType = 'arraybuffer';
    socket.onopen = () => {
      socket.send(encodeHello(campaign, `dev:${user}`));
      connection = 'open (waiting for snapshot)';
      showStatus();
    };
    socket.onmessage = (ev) => {
      if (!(ev.data instanceof ArrayBuffer)) return;
      try {
        renderer.applyServerMessage(new Uint8Array(ev.data));
        if (connection !== 'connected') {
          connection = 'connected';
          showStatus();
        }
      } catch (e) {
        fail(e);
      }
    };
    socket.onclose = () => {
      connection = 'closed (retrying in 2 s)';
      showStatus();
      setTimeout(connect, 2000);
    };
    socket.onerror = () => {};
  }
  connect();

  const frame = () => {
    try {
      renderer.render();
    } catch (e) {
      fail(e);
      return; // stop the loop on a render error rather than spamming
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

main().catch(fail);
