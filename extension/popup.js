const BASE = 'http://127.0.0.1:27124';

function setStatus(text, type) {
  const el = document.getElementById('status');
  el.textContent = text;
  el.className = type || '';
}

async function getToken() {
  const { token } = await chrome.storage.local.get('token');
  if (token) return token;
  const r = await fetch(`${BASE}/api/v1/handshake`, { method: 'POST' });
  if (!r.ok) throw new Error(`handshake ${r.status}`);
  const data = await r.json();
  await chrome.storage.local.set({ token: data.token });
  return data.token;
}

async function loadFolders(token) {
  const row    = document.getElementById('folder-row');
  const select = document.getElementById('folder-select');
  try {
    const r = await fetch(`${BASE}/api/v1/folders`, {
      headers: { 'X-UA-Token': token },
    });
    if (!r.ok) return;
    const folders = await r.json();
    if (!Array.isArray(folders) || !folders.length) return;
    folders.forEach(f => {
      const opt = document.createElement('option');
      opt.value = f.id;
      opt.textContent = f.title;
      select.appendChild(opt);
    });
    const inbox = folders.find(f => f.title === 'Новые ссылки');
    select.value = inbox ? inbox.id : folders[0].id;
    row.style.display = '';
  } catch (e) {
    // silent — select stays hidden, bookmark goes without folder id
  }
}

async function init() {
  const btn   = document.getElementById('btn-add');
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const url   = tab?.url   || '';
  const title = tab?.title || '';

  document.getElementById('url-text').textContent = url;

  let token;
  try {
    token = await getToken();
  } catch (e) {
    setStatus('URL Album не запущен', 'error');
    btn.disabled = true;
    return;
  }

  await loadFolders(token);

  btn.addEventListener('click', async () => {
    btn.disabled = true;
    setStatus('Сохраняю…');
    try {
      const folderRow = document.getElementById('folder-row');
      const sel       = document.getElementById('folder-select');
      const folderId  = (folderRow.style.display !== 'none' && sel.value)
                          ? parseInt(sel.value, 10) : NaN;
      const payload   = { url, title };
      if (!isNaN(folderId)) payload.folder_id = folderId;
      const r = await fetch(`${BASE}/api/v1/bookmarks`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-UA-Token': token },
        body: JSON.stringify(payload),
      });
      if (r.status === 401) {
        await chrome.storage.local.remove('token');
        setStatus('Ошибка авторизации', 'error');
      } else if (!r.ok) {
        setStatus(`Ошибка ${r.status}`, 'error');
      } else {
        setStatus('Добавлено ✓', 'ok');
      }
    } catch {
      setStatus('URL Album не запущен', 'error');
    }
    btn.disabled = false;
  });
}

init();
