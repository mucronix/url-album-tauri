# URL Album — архив истории разработки

Полная история ранних сессий и выпущенных релизов (2.2.0, 2.2.1, 2.2.2-beta).
Текущая разработка и накопитель невышедшего релиза — в CLAUDE.md.

---

**Сессия 1** (до 2026-05-15) — Multi-DB (`switch_db`, `last_db.txt`), compact list view в правой панели, resizable columns/splitter, DnD с `move_node` (circular ref validation), `normalize_url`, `CREATE_NO_WINDOW`.

**Сессия 2** (2026-05-15–16) — Полная система favicon (4 стратегии, domain dedup, `sameIds`, прогресс-панель); рефакторинг Tree UX (`[+]/[-]`, поведение кликов, PNG-иконки папок); grid single click → detail view; `open_url` → rundll32; `refresh_thumb` с настраиваемыми размером/таймаутом.

**Сессия 3** (2026-05-17–18) — Tree UX доработки (`noTreeExpand`, правый клик в дереве/гриде); меню "Вид" → один toggle-пункт с синхронизацией toolbar (`_syncExpandToggleUI`); меню закрывается при `window.blur`.

**Сессия 4** (2026-05-19) — Batch thumbnail refresh (`#thumb-panel`, `startThumbBatch`, `_applyThumbToCard`); `refresh_thumb` → `async fn` + `spawn_blocking`; уникальный `--user-data-dir` per invocation.

**Сессия 5** (2026-05-19) — Реструктуризация меню (Файл / Ссылки / **Перенос** / Поиск / Вид); `close_db`, `get_recent_dbs`, диалог "Свойства базы" (`#dbprops-overlay`); обновлённое контекстное меню папки с импортом в папку.

**Сессия 6** (2026-05-19) — Fix: favicon batch не обновлял UI; fix: скриншоты зависали на недоступных сайтах (`spawn` + `try_wait` poll вместо `status()`).

**Сессия 7** (2026-05-19) — Fix: DnD не работал (`dragDropEnabled: false`); DnD переписан на event delegation.

**Сессия 8** (2026-05-21) — Тестирование Win7: WebView2 несовместим (`ProcessPrng` из Win8+), минимум Windows 10. README обновлён.

**Сессия 9** (2026-06-03) — Новый репо `skljar/url-album-tauri`; удалена вкладка "Прокси"; фича "Импорт из другой базы" (`analyze_import_db`/`execute_import_db`, BFS + dedup по URL); настройка размера шрифта (`--ui-font`).

**Сессия 10** (2026-06-04) — DnD в корень: `move_node(Option<i64>)`, `#tree-root-drop`, `virtualRootId`.

**Сессия 11** (2026-06-04) — Статусбар (`#statusbar`, `setStatus`/`updateStatusLeft`, интеграция в tree/grid/search/batch).

**Сессия 12** (2026-06-04) — Удалён пункт "Файл → Выход" (`window.close()` → about:blank).

**Сессия 13** (2026-06-04) — Fix: путаница скриншотов при concurrency > 1 → `{id}_{ms}.png`, `MAX_THUMB_CONCURRENCY = 1`; fix: batch не обновлял UI; релиз **v2.1.1-beta**.

### Сессия 14 (2026-06-05) — Браузерное расширение
49. **Рефакторинг `refresh_thumb` → `do_screenshot`** — логика скриншота вынесена в `async fn do_screenshot(data_dir, id, url, width, height, timeout)` без `tauri::State`. `refresh_thumb` стала тонкой обёрткой. Заодно исправлен UI-баг: `buildTree()` делал `{ ...n, children: [] }` (shallow copy) — мутации `allNodes[i].thumb` не проникали в замыкания click-обработчиков дерева. Фикс: `n.children = []; map.set(n.id, n)` — ссылки на оригинальные объекты.
50. **HTTP-сервер** — добавлены `tiny_http = "0.12"`, `getrandom = "0.2"`. Токен генерируется при старте в `load_or_init_token()`, хранится в `settings.json` как `extensionToken`. JS: `appSettings.extensionToken = ''` чтобы токен выживал при `saveAppSettings()`. Константа `INBOX_FOLDER_NAME = "Входящие"`, хелпер `find_or_create_inbox_folder(conn)`.
    - `SERVER_PORT = 27124`, `respond_json(req, status, body, cors)`, `run_http_server(handle, token, port)` — `POST /api/v1/bookmarks`, проверка Origin + `X-UA-Token`, INSERT в "Входящие", два события: `bookmark-added` (сразу) и `thumb-updated` (после скриншота).
    - JS: листенеры `bookmark-added` → `refreshTree()` и `thumb-updated` → `_applyThumbToCard`.
    - **Fix компиляции:** `spawn_blocking` убран — `h2.state::<AppState>().db.lock()` inline в async-блоке. `dyn Read` trait object не требует `use std::io::Read` в scope (vtable dispatch). Протестировано curl: `POST /api/v1/bookmarks` → папка "Входящие", ссылка, скриншот — всё появляется в UI через события `bookmark-added` / `thumb-updated`.

51. **Браузерное расширение Chrome/Edge — ГОТОВО, работает** (коммит `79db763`):
    - **ID расширения: `imekfalcnffmmmabcjapmihbocjabecf`** — зафиксирован через `"key"` в `manifest.json` (RSA-2048 SPKI, base64). Генератор: `extension-keys/keygen.js` (Node.js crypto, `generateKeyPairSync`). Приватный ключ `extension-keys/private.key.pem` — **не в git** (`extension-keys/` в `.gitignore`), нужен для восстановления того же ID — не потерять.
    - **`POST /api/v1/handshake`** (не GET!) — браузер не шлёт заголовок `Origin` на простой GET-запрос от расширения → 403. На POST шлёт. Origin-gated константой `ALLOWED_ORIGIN = "chrome-extension://imekfalcnffmmmabcjapmihbocjabecf"`. Отдаёт `{token}` расширению, оно хранит в `chrome.storage.local`. Пользователь токен не вводит — всё автоматически.
    - **`POST /api/v1/bookmarks`** — Origin check + `X-UA-Token` + CORS (конкретный origin, не `*`). `OPTIONS` preflight обрабатывается `respond_cors_preflight()`.
    - **`extension/`**: `manifest.json` (MV3, `host_permissions: 127.0.0.1:27124`), `popup.html`, `popup.js`. При открытии popup: читает токен из storage → если нет, делает handshake → берёт url+title активной вкладки → кнопка "Добавить".
    - **Урок диагностики:** `eprintln!` не виден в GUI-сборке (`windows_subsystem = "windows"`); F12/DevTools в релизе отключены. Для отладки: писать в файл рядом с exe (`OpenOptions::append`), или смотреть DevTools popup расширения (правый клик на popup → Инспектировать → консоль, `location.origin` покажет реальный Origin).

### Сессия 15 (2026-06-06) — Релиз v2.2.0-beta
52. **Релиз v2.2.0-beta** — первый публичный релиз с браузерным расширением.
    - Версия поднята до `2.2.0-beta` в `tauri.conf.json` и `Cargo.toml` (строгий semver для cargo).
    - Архив `URL-Album-2.2.0-beta.zip` (3.8 MB): `URL-Album.exe` + `README.txt` (UTF-8) + папка `extension\` в корне.
    - Тег `v2.2.0-beta`, помечен как **pre-release** на GitHub (`--prerelease`).
    - **Раздача расширения через ZIP** — пользователь загружает "распакованное" в браузер. ID расширения одинаковый у всех (`"key"` в `manifest.json`), поэтому Origin-проверка проходит у каждого. В магазин пока не публикуем.
    - **Ссылка в README — ПРЯМАЯ:** `/releases/download/v2.2.0-beta/URL-Album-2.2.0-beta.zip`, НЕ `/releases/latest/download/...` — `latest` не видит pre-release (даёт 404). При следующем релизе ссылку в README.md нужно обновлять вручную.
    - **Приватный ключ** `extension-keys/private.key.pem` — не в git (`extension-keys*/` в `.gitignore`). Нужен для восстановления того же ID расширения — не терять.
53. **Известный баг (живое дерево при bookmark-added):** ссылка из расширения сначала появляется в корне левой панели, а не в папке «Входящие»; папка «Входящие» со ссылкой появляется только после перезагрузки базы. Похоже на проблему живого обновления дерева (`refreshTree` не раскрывает нужную папку). Не критично, чинить в следующей сессии. **(исправлен в п.69, 2026-06-12)**

### Сессия 16 (2026-06-07) — Релиз v2.2.1-beta
54. **Переименование папки по умолчанию:** `INBOX_FOLDER_NAME = "Входящие"` → `"Новые ссылки"`. Константа в `main.rs:16`, логика `find_or_create_inbox_folder` не менялась.
55. **Новый эндпоинт `GET /api/v1/folders`** — список корневых папок (`kind='folder' AND parent IS NULL ORDER BY sort_idx`), возвращает `[{"id": N, "title": "..."}]`.
    - Защита: Origin-проверка **ослаблена** — браузер не шлёт `Origin` на простые GET-запросы без кастомных заголовков. Логика: если Origin прислан и не совпадает с `ALLOWED_ORIGIN` → 403; если Origin пустой или совпадает → пропускаем. Основная защита — токен `X-UA-Token`.
    - `Access-Control-Allow-Origin` в ответе всегда `Some(ALLOWED_ORIGIN)` (независимо от `cors`).
    - `POST /api/v1/bookmarks` и `POST /api/v1/handshake` — не тронуты.
56. **Расширение — выбор папки в popup:**
    - `GET /api/v1/folders` вызывается при открытии popup после получения токена; заполняет `<select id="folder-select">`.
    - По умолчанию выбирается папка с `title === 'Новые ссылки'`; если её нет — первая в списке.
    - Если запрос не удался или список пустой — select скрыт, ссылка уходит без `folder_id` (прежнее поведение).
    - `folder_id` (parseInt из select) добавляется в тело `POST /api/v1/bookmarks` только если select виден и значение числовое.
57. **`POST /api/v1/bookmarks` — необязательный `folder_id`:**
    - Читается из тела запроса как `v["folder_id"].as_i64()`.
    - Проверяется через `SELECT id FROM nodes WHERE id=?1 AND kind='folder'`.
    - Если папка найдена → использует её; иначе → фолбэк на `find_or_create_inbox_folder` («Новые ссылки»).
    - Скриншот в фоне и событие `bookmark-added` — не тронуты.
58. **Версия → 2.2.1-beta:** `Cargo.toml`, `tauri.conf.json` → `2.2.1-beta`; `extension/manifest.json` → `2.2.1` (Chrome не поддерживает `-beta` в версии манифеста).
    - **Известный баг (живое дерево при `bookmark-added`) — ссылка из расширения появляется в корне, а не в нужной папке; правильно отображается после перезагрузки базы. (исправлен в п.69, 2026-06-12)**
59. **Fix: дерево сворачивалось после обновления favicon.** `_finishFaviconBatch()` вызывал `renderTree()` напрямую, без сохранения состояния раскрытых папок → все папки схлопывались. Исправлено: обёрнуто в `saveOpenState()` / `restoreOpenState()`, как в `refreshTree()`. (`_finishThumbBatch` дерево не трогает — там проблемы не было.) Архив релиза v2.2.1-beta перевыпущен с фиксом, релиз помечен Latest (снят флаг prerelease).
60. **Fix (после релиза 2.2.1): зона дропа в корень перекрывала верхнюю папку.** `#tree-root-drop` была position:absolute (накладывалась поверх первой папки, мешая дропу в неё). Переделано: зона всегда в потоке, по умолчанию opacity:0 + pointer-events:none, при drag — opacity:1 (вариант, не меняющий DOM, чтобы не сорвать drag в WebView2). Высота согласована с --ui-font через padding/line-height как у .tree-item. Дроп ссылок и папок в корень оставлен как был. Вошло в релиз 2.2.2-beta.
61. **Feat: автопрокрутка дерева при drag.** При перетаскивании ссылки/папки к верхнему/нижнему краю #tree список автоматически прокручивается, чтобы дотянуться до элементов вне видимости. Реализация: в treeEl.dragover вычисляется зона 40px у краёв (getBoundingClientRect + e.clientY), направление пишется в _scrollDir (-1/0/+1), цикл requestAnimationFrame (_scrollRafId) крутит scrollTop на 8px/кадр. Остановка — _stopAutoScroll() в dragend (3 источника) и drop (rootZone/tree/grid). НЕ трогает _clearDragOver (иначе скролл прерывался при смене папки под курсором). scrollTop не меняет DOM → drag в WebView2 не срывается.
62. **Релиз 2.2.2-beta выпущен (08.06.2026).** Версия поднята в Cargo.toml/tauri.conf.json (2.2.2-beta) и extension/manifest.json (2.2.2). README.md и README.txt обновлены; раздел «ИСТОРИЯ ВЕРСИЙ» в README.txt переоформлен в формат + / ! и дополнен версиями 2.0/2.1/2.1.1. Архив URL-Album-2.2.2-beta.zip (URL-Album.exe + README.txt + extension\, без extension-keys) опубликован на GitHub, помечен Latest. В релиз вошли: фикс зоны дропа в корень (п.60) и автопрокрутка дерева (п.61).
