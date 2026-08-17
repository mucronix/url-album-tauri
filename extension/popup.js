const BASE = 'http://127.0.0.1:27124';

// Тексты статуса собраны в одном месте: одни и те же строки показывают и
// загрузка списка папок, и отправка закладки.
const MSG_NO_APP      = 'URL Album не запущен';
const MSG_RECONNECTED = 'Программа перезапущена, подключение обновлено — нажмите «Добавить»';
const MSG_NO_FOLDERS  = 'Список папок недоступен — закладка уйдёт в «Новые ссылки»';
const MSG_NO_AUTH     = 'Закладка не сохранена. Закройте и откройте это окно';

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

/// Заполнить список папок. Возвращает 'ok' | 'unauthorized' | 'offline' |
/// 'unavailable'; текст статуса показывает вызывающий — он знает, чем занят.
///
/// 'offline' и 'unavailable' — РАЗНЫЕ вещи, склеивать их нельзя: первое значит
/// «сервера на порту нет вовсе» (программа закрыта), второе — «сервер ответил,
/// но ответ негодный». На склейке уже споткнулись: при закрытой программе окно
/// сообщало «закладка уйдёт в «Новые ссылки»», хотя уйти ей было некуда.
///
/// Раньше функция глотала любой отказ (`if (!r.ok) return;` и пустой `catch`):
/// список папок просто исчезал, и человек не понимал, куда попадёт ссылка.
/// Это половина симптома «первый клик даёт ошибку авторизации».
async function loadFolders(token) {
  const row    = document.getElementById('folder-row');
  const select = document.getElementById('folder-select');

  let r;
  try {
    r = await fetch(`${BASE}/api/v1/folders`, { headers: { 'X-UA-Token': token } });
  } catch {
    return 'offline';        // fetch не состоялся: на порту никто не слушает
  }
  // 401 = на порту другая копия программы: у запущенной из другой папки свой
  // settings.json и свой токен.
  if (r.status === 401) return 'unauthorized';
  if (!r.ok)            return 'unavailable';

  let folders;
  try { folders = await r.json(); } catch { return 'unavailable'; }
  if (!Array.isArray(folders) || !folders.length) return 'unavailable';

  // Очистка обязательна: функция теперь вызывается повторно, после обновления
  // подключения, а `forEach` ниже только добавляет — список бы удвоился.
  select.innerHTML = '';
  folders.forEach(f => {
    const opt = document.createElement('option');
    opt.value = f.id;
    opt.textContent = f.title;
    select.appendChild(opt);
  });
  const inbox = folders.find(f => f.title === 'Новые ссылки');
  select.value = inbox ? inbox.id : folders[0].id;
  row.style.display = '';
  return 'ok';
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
    setStatus(MSG_NO_APP, 'error');
    btn.disabled = true;
    return;
  }

  // Обновление подключения — не более одного раза за открытие окна: 401
  // приходит из двух мест (список папок и отправка), и без флага они
  // задёргали бы handshake по кругу.
  let reconnected = false;

  /// Токен в chrome.storage мог остаться от прошлой копии программы. Стираем
  /// его, берём новый через handshake и перезагружаем список папок. Отправку
  /// закладки НЕ повторяем: смена токена — единственный признак того, что
  /// закладка уйдёт в другую базу, и нажать «Добавить» человек должен
  /// осознанно.
  ///
  /// Возвращает 'ok' | 'gone' (программа не ответила, кнопка выключена)
  /// | 'skipped' (в этом окне уже обновляли).
  ///
  /// Присваивание в `token` обязано быть здесь, а не у вызывающего: обработчик
  /// клика захватил переменную замыканием и сам её не перечитывает.
  async function reconnect() {
    if (reconnected) return 'skipped';
    reconnected = true;

    await chrome.storage.local.remove('token');
    try {
      token = await getToken();          // storage пуст — уйдёт в handshake
    } catch {
      setStatus(MSG_NO_APP, 'error');
      btn.disabled = true;
      return 'gone';
    }

    // После успешного handshake список папок почти наверняка загрузится — мы
    // только что говорили с этой копией. Поэтому главным оставляем сообщение
    // о переподключении: оно объясняет, почему список поменялся.
    // Но если программа закрылась между handshake и списком папок, «подключение
    // обновлено» было бы показом успеха, которого нет.
    if (await loadFolders(token) === 'offline') {
      setStatus(MSG_NO_APP, 'error');
      btn.disabled = true;
      return 'gone';
    }
    setStatus(MSG_RECONNECTED);
    return 'ok';
  }

  const foldersState = await loadFolders(token);
  if (foldersState === 'unauthorized') {
    await reconnect();
  } else if (foldersState === 'offline') {
    // Программа закрыта. Прежде это выяснялось только после клика, и лишь
    // потому, что падал POST: при токене в storage окно молчало вовсе, а
    // handshake не выполнялся — токен же был. Кнопку выключаем, нажимать её
    // бессмысленно, POST уйдёт в ту же пустоту.
    setStatus(MSG_NO_APP, 'error');
    btn.disabled = true;
  } else if (foldersState === 'unavailable') {
    setStatus(MSG_NO_FOLDERS);
  }

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
        const res = await reconnect();
        // 'gone': программы больше нет, reconnect уже сказал об этом и выключил
        // кнопку — выходим ДО её включения в конце обработчика.
        if (res === 'gone') return;
        if (res === 'skipped') setStatus(MSG_NO_AUTH, 'error');
      } else if (!r.ok) {
        setStatus(`Ошибка ${r.status}`, 'error');
      } else {
        setStatus('Добавлено ✓', 'ok');
      }
    } catch {
      setStatus(MSG_NO_APP, 'error');
    }
    btn.disabled = false;
  });
}

init();
