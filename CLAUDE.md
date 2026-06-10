# URL Album 2 — CLAUDE.md

Контекст для продолжения работы в новом окне.

---

## Что это за проект

**URL Album 2** — portable desktop bookmark manager на Tauri 2 + Rust + Vanilla JS.  
Духовный наследник старого URL-Album (Win32, ~2008). Хранит закладки локально в SQLite,  
без облака, без синхронизации. Философия: portable, minimalistic, classic Win32 UX.

Оригинальный URL-Album (`urlalbum.exe`, `ua.dat.bak`) лежит в корне проекта для сравнения.

---

## Структура проекта

```
C:\Projects\url-album-tauri\
├── src-tauri\               ← Rust/Tauri backend
│   ├── src\
│   │   ├── main.rs          ← все Tauri-команды (~1700+ строк)
│   │   ├── db.rs            ← SQLite схема, запросы, экспорт/импорт
│   │   └── importer.rs      ← парсер ua.dat (Windows-1251)
│   ├── Cargo.toml
│   ├── tauri.conf.json      ← frontendDist: "../ui", center:true, minWidth:500
│   └── build.rs
├── ui\                      ← Vanilla JS frontend (встраивается в exe при сборке)
│   ├── index.html
│   ├── app.js               ← весь UI (~4800+ строк)
│   ├── style.css
│   └── icons\               ← PNG иконки (встраиваются в exe)
│       ├── folder-closed.png  ← пиксельная иконка закрытой папки
│       └── folder-open.png    ← пиксельная иконка открытой папки
├── CLAUDE.md                ← этот файл
├── docs\superpowers\
│   ├── specs\               ← design docs
│   └── plans\               ← implementation plans
└── Data\                    ← thumbnails/скриншоты закладок (legacy)
```

---

## Как запустить

**⚠️ Важно**: Tauri встраивает `ui/` в бинарник при компиляции.  
**Любые изменения в JS/CSS/HTML требуют `cargo build`** перед запуском.  
Простой перезапуск exe без rebuild = старый встроенный frontend.

```powershell
# Рабочая директория: C:\Projects\url-album-tauri\src-tauri

# ⚠️ Сначала убить процесс — иначе cargo не может заменить exe (access denied)
Stop-Process -Name "url-album" -Force -ErrorAction SilentlyContinue

# Собрать и запустить (debug):
cargo build
Start-Process ".\target\debug\url-album.exe" -WorkingDirectory ".\target\debug"

# Release сборка:
cargo build --release

# ❌ НЕ использовать cargo tauri dev — бинарник требует запущенного dev-сервера
```

Portable-файлы рядом с exe (в `target\debug\`):
- `album.db` — база по умолчанию
- `last_db.txt` — последняя открытая база (авто-resume при старте)
- `settings.json` — настройки приложения
- `toolbar.json` — конфиг тулбара
- `browsers.json` — список браузеров
- `Data\` — скриншоты/thumbnails + `Data\favicons\` — кэш favicon файлов

---

## Стек

| Компонент | Технология |
|---|---|
| Shell | Tauri 2 |
| Backend | Rust |
| БД | SQLite (rusqlite, bundled), WAL mode, `PRAGMA synchronous = FULL` |
| Frontend | Vanilla JS (без фреймворков), CSS переменные, HTML5 DnD |
| Диалоги | rfd 0.15 (`AsyncFileDialog`, без `set_parent` — DPI-бага на Windows) |
| HTTP | reqwest 0.12 (rustls-tls, async) |
| Encoding | encoding_rs (Windows-1251 для ua.dat) |

---

## Что реализовано и работает

### Backend (Rust / main.rs)
- `get_tree` — дерево всех узлов (папки + ссылки), включает поле `favicon`
- `get_bookmarks` — ссылки папки
- `create_bookmark(parent_id, title, url, note?)` — создать ссылку
- `create_folder(parent_id, title)` — создать папку
- `update_bookmark(id, title, url, note)` — редактировать ссылку
- `rename_node(id, title)` — переименовать папку
- `delete_folder(id)` — рекурсивное удаление с CTE
- `delete_node(id)` — удалить ссылку
- `move_node(id, new_parent)` — drag & drop, с валидацией circular refs
- `set_sort_idx(id, sort_idx)` — порядок сортировки
- `sort_folder(folder_id, by, desc)` — сортировка папки
- `sort_all_bookmarks(by, desc)` — глобальная сортировка
- `search_bookmarks(query, by_title, by_url, by_note)` — поиск (папки + ссылки)
- `open_url(url)` — открыть URL в браузере (`rundll32.exe url.dll,FileProtocolHandler`)
- `open_file(path)` — открыть локальный файл в программе по умолчанию (`cmd /c start`)
- `open_url_with(url, browser)` — открыть в конкретном браузере
- `check_url(url)` — HTTP-проверка ссылки
- `create_new_db` — создать новую БД (Save File Dialog)
- `open_db` — открыть существующую БД
- `switch_db` — переключиться между базами, checkpoint WAL
- `get_db_path` / `set_window_title` — путь к активной БД, titlebar
- `get_data_dir` — путь к папке Data/ рядом с exe (используется JS для favicon путей)
- `backup_db` / `backup_db_with_data` — резервная копия
- `clear_db` — очистить базу (с VACUUM + WAL restore)
- `checkpoint_db` — WAL checkpoint
- `refresh_thumb(id, url, width?, height?, timeout?)` — скриншот через Edge/Chrome headless; принимает размер и таймаут из настроек
- `clear_thumb(id)` / `clear_screenshots()` — очистить thumbnails
- `fetch_favicon(id, url)` — загрузить favicon: кэш → favicon.ico → HTML `<link>` → DuckDuckGo → Google; `is_valid_image()` отсеивает HTML-ошибки; cache validation (перезагружает битые файлы)
- `update_node_favicon(id, filename)` — записать favicon filename в DB (для sameIds domain dedup)
- `import_uadat / import_uadat_pick` — импорт из старого ua.dat
- `import_html / import_txt / import_sync` — импорт из HTML/TXT/JSON
- `import_from_browser(browser_id)` — импорт из Chrome/Firefox/Edge/Opera/Brave
- `import_from_bookmarks_file` — импорт из конкретного файла
- `export_folder_html / export_folder_txt / export_folder_sync` — экспорт
- `detect_browsers / detect_browser_exes` — автодетектирование браузеров
- `load_browsers_config / save_browsers_config` — portable browsers.json
- `load_settings / save_settings` — portable settings.json
- `load_toolbar_config / save_toolbar_config` — portable toolbar.json
- `normalize_url(url)` — добавляет https:// если нет схемы (только при открытии, не в БД)
- `analyze_import_db(window)` → `ImportAnalysis` — открывает File Dialog, читает исходную БД read-only, считает новые/дубли по нормализованному URL; **не меняет текущую БД**
- `execute_import_db(path, dest_parent?)` → `usize` — вставляет только новые ссылки, воссоздаёт нужные папки BFS top-down с перемаппингом id
- `db::collect_urls(conn)` — HashSet нормализованных URL текущей БД
- `db::normalize_url_for_dedup(url)` — убирает схему, www., trailing slash, lowercase; используется для дедупликации

### Favicon helpers (Rust / main.rs)
- `extract_domain(url)` — извлечь домен, убрать www.
- `sanitize_domain(domain)` — только `[a-z0-9.-]`, остальное → `_`
- `ext_from_content_type(ct)` — определить расширение по Content-Type
- `is_valid_image(bytes)` — проверка magic bytes (PNG, ICO, GIF, JPEG, WebP, SVG)
- `find_icon_href(html, base)` — найти `<link rel="icon">` в HTML
- `attr_value(tag, attr)` / `resolve_href(href, base)` — вспомогательные для парсинга

### Multi-DB / Portable
- `AppState { db: Mutex<Connection>, db_path: Mutex<PathBuf> }`
- `last_db.txt` рядом с exe — авто-resume последней базы при старте
- Все пути относительны exe: `album.db`, `Data\`, конфиги
- Диалоги открытия/создания БД стартуют в папке текущей базы
- Полностью portable: в реестр ничего не пишется; `reg query` используется только read-only для автодетектирования браузеров

### DB Schema
```sql
CREATE TABLE nodes (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    parent   INTEGER,
    kind     TEXT NOT NULL DEFAULT 'bookmark',
    title    TEXT NOT NULL,
    url      TEXT,
    thumb    TEXT,         -- полный абсолютный путь к screenshot PNG
    note     TEXT,
    created  TEXT,
    visited  TEXT,
    sort_idx INTEGER DEFAULT 0,
    favicon  TEXT          -- только filename (напр. "github.com.png"), путь собирается в runtime
);
```

### Frontend (app.js / ~4900+ строк)

**Навигационная модель:**
- Левая панель: дерево (папки + ссылки как листья, `●` иконка или favicon 16×16)
- Правая панель: содержимое папки (подпапки + ссылки) как compact list view
- Два режима: list mode (grid) и viewer mode (detail view)
- Папки **всегда выше ссылок** на каждом уровне дерева и в гриде

**Поведение кликов — дерево:**
- Клик на `[+]/[-]` — только toggle open/close (НЕ влияет на выделение, НЕ закрывает другие папки)
- Клик на название папки — выделение серым + показ содержимого в гриде (без toggle, без accordion)
- Двойной клик на название папки — toggle open/close (только эта папка, остальные не трогает)
- Правый клик на папку — выделяет и показывает контекстное меню
- Клик на ссылку — `selectTreeBookmark` → `openDetailView`
- ↑↓ стрелки — перемещают фокус И выбирают элемент

**Важно:** `selectFolder(id, expand, noTreeExpand)`:
- `expand=false, noTreeExpand=true` — только выделение+грид, ничего не раскрывает и не закрывает
- Accordion mode (`collapseSiblingBranches`) намеренно убран из всех click/dblclick обработчиков дерева

**При старте:** все папки закрыты, первая папка только подсвечивается (не раскрывается)

**Поведение кликов — грид:**
- Single click → ссылка: `openDetailView(node)` — full viewer (карточка)
- Double click → ссылка: `openWithBrowser(url)` — открыть в браузере
- Single/double click → папка: `selectFolder(id)` — navigate into folder
- Правый клик → папка: `showFolderContextMenu` (работает и в гриде, и в дереве)

**Компоненты UI:**
- `#sidebar` + `#splitter` + `#main` — основной layout, splitter resizable (сохраняется в settings)
- `#list-header` + `#grid` — compact list с resizable колонками (CSS var `--col-name-w`)
- `#detail-view` — full viewer: thumbnail + favicon перед URL + note
- `#favicon-panel` — non-modal прогресс-панель загрузки favicon (левый нижний угол)
- `#breadcrumb` — путь к текущему элементу
- Toolbar с кастомизацией (`CMD_REGISTRY`, drag & drop порядка кнопок)
- Menubar: Файл, Ссылки, Поиск, Вид
- Поиск: Ctrl+F, ищет по папкам + названиям + URL + заметкам
- Меню закрывается при `window.blur` (клик на titlebar, Alt+Tab)

**Дерево — визуальные элементы:**
- `[+]/[-]` кнопки (CSS `::before` на `span.arrow[data-has-children]`) — toggle open/close
- Иконки папок: `_makeFolderSvg(open)` — два `<img>` из `ui/icons/`, CSS переключает по `.open`
  - `icons/folder-closed.png` — закрытая папка (pixel-art стиль)
  - `icons/folder-open.png` — открытая папка (pixel-art стиль)
  - `.fsvg-closed` / `.fsvg-open` — классы для CSS-переключения
- Выделение: серый фон только на `.label` (не вся строка)
- Ссылки: favicon иконка или `●` + label
- Сортировка в меню: один пункт на поле, toggle asc/desc при повторном клике (▲/▼ индикатор)
- Меню "Вид": один пункт "Развернуть/Свернуть все папки" — toggle, синхронизирован с toolbar кнопкой через `_syncExpandToggleUI()` (обновляет текст+иконку в обоих местах)
- `group.dataset.id = menu.id` — для идентификации меню при открытии (вызов sync при открытии "Вид")

**Favicon система (JS):**
- `MAX_FAVICON_CONCURRENCY = 5` — intentional rate limiting
- `dataDir` — путь к Data/ (через `get_data_dir`); `faviconFilePath(filename)` — нормализует separators для `convertFileSrc`
- `buildFaviconQueue` + `sameIds` — dedup по домену; `startFaviconBatch(folderNode, recursive)` / `loadSingleFavicon(node)` — entry points
- `applyFaviconToDOM` / `updateFaviconInDOM` — live update tree + grid + detail

**Контекстное меню ссылки:**
Открыть → Открыть с помощью → [sep] → Открыть рисунок → Обновить рисунок → Удалить рисунок → [sep] → Загрузить favicon → [sep] → Удалить ссылку → [sep] → Копировать URL → Свойства

**Контекстное меню папки:**
Экспорт → [sep] → Сортировка (toggle asc/desc) → [sep] → Проверить → Загрузить favicon'ы → Переименовать → [sep] → Удалить → [sep] → Свойства

**Диалоги:**
- "Новая ссылка": поля URL, Название, Заметка. НЕ закрывается по backdrop-клику
- "Свойства ссылки": OK / Отмена
- "Свойства папки": OK / Отмена
- "Дубликаты ссылок" — full-screen двухпанельный finder
- "Браузер-менеджер" — portable browsers.json
- "Настройки" — вкладки: Общие, Рисунок (кнопка "По умолчанию": 1280×800, 15сек)
- "Импорт из другой базы" — диалог статистики (папок/закладок/новых/дубликатов) + select назначения

**Сохраняемые настройки (settings.json):**
- `theme` — light/dark
- `showToolbar` — видимость toolbar
- `listColWidth` — ширина колонки "Название" в grid (%)
- `sidebarWidth` — ширина левой панели (px)
- `accordionTree` — accordion режим дерева
- `confirmDelete` — подтверждение удаления
- `noDuplicateUrls` — проверка дублей при добавлении
- `thumbWidth` / `thumbHeight` / `thumbTimeout` — настройки скриншота (дефолт: 1280×800, 15сек)

---

## Известные баги / TODO

### Активные проблемы
- [ ] Accordion mode в настройках — не всегда корректно закрывает ветки при навигации из правой панели
- [ ] Поиск — breadcrumb не всегда обновляется при клике на папку из результатов
- [ ] `backup_db` с `set_parent(&window)` — может вызывать DPI issues на Windows

### Архитектурные ограничения
- `rfd::AsyncFileDialog` без `set_parent` на Windows (убрано из `open_db` из-за DPI-бага)
- `allNodes` — полная перезагрузка при каждом изменении через `invoke('get_tree')`
- `thumb` хранит полный абсолютный путь в DB (legacy, в отличие от `favicon` который хранит только filename)

### Что НЕ сделано / очередь
- [ ] Drag-сортировка в ДЕРЕВЕ (в гриде работает, п.66)
- [ ] `thumb` хранит абсолютный путь в DB → перейти на filename как у `favicon` (при переносе папки скриншоты ломаются)
- [ ] Browser import (`import_chromium`, `import_firefox`) → добавить `parent_id` (сейчас всегда в корень)
- [ ] Favicon: очистка orphaned файлов из `Data/favicons/` при удалении закладок
- [ ] Массовое выделение / batch operations
- [ ] Favicon: force refresh / TTL (YAGNI пока)
- [ ] **Локализация (i18n)** — крупная задача, делать отдельной сессией (см. ниже)

### DnD — состояние (проверено 2026-06-04)
- Защита от циклов: **двойная** — JS `_isDragValid` (walk up через `allNodes`) + Rust `move_node` (walk up через БД). Потеря данных невозможна.
- Сохранение в БД: `UPDATE nodes SET parent = ?1` при каждом drop — персистируется.
- Drop в корень: **реализован** — `move_node(Option<i64>)`, `WHERE parent IS ?1` (SQLite IS для NULL), `#tree-root-drop` drop-зона.
- **Важно: `#tree-root-drop` — `position: absolute`**, не в потоке. Если сделать `display: block` в нормальном потоке во время `dragstart`, sidebar reflow сдвигает drag-source → Chromium/WebView2 отменяет drag немедленно.
- `virtualRootId` (JS) — ID папки-обёртки legacy-баз; если есть, drop в корень → эта папка, иначе `parent = NULL`.
- `body.is-dragging` — класс на `<body>` при активном drag (dragstart/dragend всех трёх источников: tree-item, grid-row, grid-card). Управляет видимостью `#tree-root-drop` через CSS.
- Drag-сортировка ссылок и папок в гриде (порядок + вложение по зонам, п.66).

### Крупные задачи (отдельная сессия)

#### Локализация (i18n)
**Цель:** вынести все строки интерфейса в отдельные файлы по языкам (`ru.json`, `en.json`), чтобы пользователи могли сами переводить на любой язык копированием файла.

**Что нужно сделать:**
- Найти ВСЕ UI-строки в JS: меню, кнопки, контекстные меню, диалоги, статусы, подсказки, tooltips
- Найти строки в Rust: имя папки `"Входящие"` (`INBOX_FOLDER_NAME`), сообщения HTTP-сервера/расширения
- Вынести в файлы локализации с ключами, заменить хардкод на `t('ключ')`
- Добавить загрузку языка из `settings.json` + переключатель в настройках
- Первый язык — английский в дополнение к русскому

**Начинать с плана:** инвентаризация строк → структура файлов → формат ключей → механизм переключения → реализация.

---

## Критические ловушки

Неочевидные root causes и решения — не теряй при рефакторинге.

### Tauri / WebView2

- **`dragDropEnabled: false` в `tauri.conf.json` обязателен.** Tauri по умолчанию перехватывает все OS-level drag-события для своего механизма drop файлов в окно — это блокирует `dragstart` на всех HTML-элементах, DnD не работает вообще.

- **`window.close()` в WebView2 уводит на `about:blank`**, оставляя пустую рамку. Закрытие только через ×/Alt+F4. Пункт "Файл → Выход" удалён по этой причине.

- **`#tree-root-drop` обязан быть вне потока.** Показ/скрытие через `display: block` во время `dragstart` вызывает sidebar reflow → сдвигает drag-source → Chromium/WebView2 немедленно отменяет drag (firing `dragend`). Текущее решение: `opacity: 0/1` + `pointer-events: none/auto` — DOM не меняется, reflow нет.

- **`eprintln!`/`dbg!` не видны в GUI-сборке** (`windows_subsystem = "windows"`). DevTools в release отключены. Для отладки: `"devtools": true` в `tauri.conf.json` временно, или писать в файл (`OpenOptions::append`).

### Rust — IPC и блокировки

- **Блокирующие команды (`std::process::Command::status()` и т.п.) — обязательно `async fn` + `spawn_blocking`.** Иначе IPC-поток замерзает и UI не реагирует. Касается: `refresh_thumb`, любой команды, запускающей внешний процесс.

- **Не держать `MutexGuard` через `.await`.** В async-командах: взять `db.lock()`, выполнить запрос, отпустить guard до первого `.await`.

- **Tauri 2: направление конвертации имён.**
  - JS → Rust (аргументы `invoke`): camelCase **конвертируется** в snake_case автоматически. `invoke('cmd', { sortIdx: 5 })` доходит до параметра `sort_idx: i64`; аналогично `parentId → parent_id`, `newPath → new_path`.
  - Rust → JS (поля ответа, сериализованные структуры): **НЕ конвертируются**, остаются snake_case. Структура с полем `sort_idx` приходит в JS как `n.sort_idx`, не `n.sortIdx`. При обращении к полям ответа всегда использовать snake_case.

### SQLite / rusqlite

- **`WHERE parent IS ?1` вместо `= ?1` при работе с NULL.** SQLite: `NULL = NULL` → false; `NULL IS NULL` → true. Используется в `move_node` и фильтрации корневых узлов.

- **`sort_idx` обязан быть в SELECT и в `struct TreeNode`.** Без поля `pub sort_idx: i64` в `TreeNode` фронтенд получает `undefined` — все `.sort((a,b) => (a.sort_idx??0)-...)` тихо сводятся к сортировке по `id`. Симптом: порядок сбрасывается при каждом перезапуске, хотя в БД записано верно. Дерево "работало" случайно — V8 стабильная сортировка сохраняла порядок из `ORDER BY sort_idx,id`.

- **`thumb` хранит полный абсолютный путь (legacy); `favicon` — только filename.** Путь favicon строится в runtime: `exe_dir/Data/favicons/{filename}`. При переносе папки скриншоты ломаются — известное ограничение.

### Скриншоты (thumbnails)

- **`MAX_THUMB_CONCURRENCY = 1` — не поднимать.** При > 1 воркеры стартуют в одну секунду → одинаковый timestamp → один файл → браузеры перезаписывают друг друга → скриншоты путаются между ссылками. Имена `{id}_{ms}.png` частично решают, но ограничение остаётся.

- **Уникальный `--user-data-dir` per invocation** (`ua_screenshot_{id}`) — иначе конфликт профилей Edge/Chrome headless при параллельных вызовах.

### Браузерное расширение

- **`POST /api/v1/handshake`, не GET.** Браузер не шлёт заголовок `Origin` на простые GET-запросы — Origin-проверка всегда давала 403. POST шлёт Origin.

- **`GET /api/v1/folders` — Origin-проверка ослаблена намеренно.** Браузер не шлёт Origin на GET без кастомных заголовков → нельзя требовать совпадение. Защита — токен `X-UA-Token`. Логика: Origin прислан и не совпадает → 403; пустой → пропустить.

- **Extension ID `imekfalcnffmmmabcjapmihbocjabecf` зафиксирован через `"key"` в `manifest.json`.** Приватный ключ `extension-keys/private.key.pem` **не в git** (`extension-keys*/` в `.gitignore`) — нужен для восстановления того же ID. Не потерять.

### Релиз

- **4 места для обновления версии:** `Cargo.toml`, `tauri.conf.json`, `extension/manifest.json` (без `-beta` — Chrome не поддерживает), `APP_VERSION` в `app.js`.

- **Ссылка в README — прямая** (`/releases/download/vX.Y.Z/...`), не `/releases/latest/download/...` — `latest` не видит pre-release, даёт 404.

- **ZIP без `extension-keys/`** — в `.gitignore`, в архив не попадает. Состав: `URL-Album.exe` + `README.txt` + `extension\`.

### Поддерживаемые ОС

- **Минимум Windows 10.** WebView2 требует `ProcessPrng` из `bcryptprimitives.dll` — появилась в Windows 8+. На Win7 WebView2 (включая v109) падает с "ProcessPrng не найдена". DLL-шим отклонён — антивирусы флагают как малварь.

### JS — синхронизация дерева и грида

- **После изменения порядка/содержимого обновлять ОБА — дерево (`renderTree`) И грид (`loadFolderContents`/`loadBookmarks`) + синхронизировать `allNodes sort_idx`.** Рендерятся раздельно, легко забыть одно. Баг повторялся трижды: favicon batch не обновлял дерево (п.59), drag-сортировка папок не обновляла грид, `sort_folder` не обновлял грид (allNodes был stale). Паттерн: `allNodes[i].sort_idx = newIdx` → `renderTree` → `loadFolderContents`/`loadBookmarks`.

---

## Паттерны и соглашения

### Rust
- Все команды через `state: tauri::State<AppState>`
- `move_node`, `create_bookmark` и т.д. — параметры в snake_case (Tauri конвертирует из camelCase)
- `CREATE_NO_WINDOW (0x0800_0000)` на все `Command::new` для консольных exe
- `normalize_url()` — вызывается в open_url, open_url_with, refresh_thumb, fetch_favicon, check_url
- `open_url` использует `rundll32.exe url.dll,FileProtocolHandler` (не `cmd /c start` — ненадёжно)
- Async команды (fetch_favicon, check_url, refresh_thumb): НЕ держать MutexGuard через `.await`
- Команды с блокирующими процессами (`std::process::Command::status()`) — обязательно `async fn` + `tauri::async_runtime::spawn_blocking`, иначе IPC-поток замерзает и UI не реагирует
- `favicon` в DB: только filename (`github.com.png`), путь = `exe_dir/Data/favicons/{filename}`

### JS
- `allNodes` — in-memory кэш всего дерева, обновляется через `invoke('get_tree')`
- `allFolders` — производная от `allNodes`
- `activeFolderId` — текущая папка в grid
- `activeBookmarkNode` — выделенная ссылка (null в list mode)
- `dataDir` — путь к Data/ (без trailing slash, загружается в init())
- `faviconFilePath(filename)` — `dataDir.replace(/\\/g, '/') + '/favicons/' + filename`
- `_dragNode` — глобальное состояние DnD
- `selectFolder(id, expand=true, noTreeExpand=false)` — expand=false для tree-clicks, noTreeExpand=true чтобы не трогать состояние дерева вообще
- `raiseOverlay(el)` — перемещает overlay в конец body для правильного z-index
- `convertFileSrc(path)` — Tauri asset:// URL для локальных файлов
- Все изменения в ui/ требуют `cargo build`

### CSS
- CSS переменные: `--sidebar-w`, `--col-name-w`, `--accent`, `--bg`, `--bg2`, `--bg3`, `--border`, `--text`, `--text2`, `--text-dim`
- Light/Dark theme через `data-theme` на `<html>`
- `.dlg-overlay` z-index: 9000, `#confirm-overlay` z-index: 10000
- Grid layout для list rows: `grid-template-columns: 18px var(--col-name-w) 5px 1fr`
- `.favicon-icon` — 16×16, `image-rendering: pixelated`, `object-fit: contain`
- `#favicon-panel` — `position: fixed; bottom: 24px; left: 24px` (non-modal)
- `#thumb-panel` — аналогично, z-index: 501, перетаскивается за `#tp-titlebar`; `makeDlgDraggable` сбрасывает `bottom/right → auto` при drag
- `_applyThumbToCard(id, title, newPath)` — обновляет `allNodes` + grid card DOM; используй его при любых изменениях thumbnail
- `.tree-item .arrow[data-has-children]::before` — `+` / `.tree-item.open > .arrow[data-has-children]::before` — `−`
- `.tree-item:hover > .label` / `.tree-item.active > .label` — серый фон только на тексте
- `.fsvg-closed` / `.fsvg-open` + `.tree-item.open` — CSS переключение иконок папок
- `.folder-icon img` — `image-rendering: pixelated`, 18×18px

---

## История изменений (крупные сессии)

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
53. **Известный баг (живое дерево при bookmark-added):** ссылка из расширения сначала появляется в корне левой панели, а не в папке «Входящие»; папка «Входящие» со ссылкой появляется только после перезагрузки базы. Похоже на проблему живого обновления дерева (`refreshTree` не раскрывает нужную папку). Не критично, чинить в следующей сессии.

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
    - **Известный баг (живое дерево при `bookmark-added`) — не исправлен:** ссылка из расширения появляется в корне, а не в нужной папке; правильно отображается после перезагрузки базы.
59. **Fix: дерево сворачивалось после обновления favicon.** `_finishFaviconBatch()` вызывал `renderTree()` напрямую, без сохранения состояния раскрытых папок → все папки схлопывались. Исправлено: обёрнуто в `saveOpenState()` / `restoreOpenState()`, как в `refreshTree()`. (`_finishThumbBatch` дерево не трогает — там проблемы не было.) Архив релиза v2.2.1-beta перевыпущен с фиксом, релиз помечен Latest (снят флаг prerelease).
60. **Fix (после релиза 2.2.1): зона дропа в корень перекрывала верхнюю папку.** `#tree-root-drop` была position:absolute (накладывалась поверх первой папки, мешая дропу в неё). Переделано: зона всегда в потоке, по умолчанию opacity:0 + pointer-events:none, при drag — opacity:1 (вариант, не меняющий DOM, чтобы не сорвать drag в WebView2). Высота согласована с --ui-font через padding/line-height как у .tree-item. Дроп ссылок и папок в корень оставлен как был. Вошло в релиз 2.2.2-beta.
61. **Feat: автопрокрутка дерева при drag.** При перетаскивании ссылки/папки к верхнему/нижнему краю #tree список автоматически прокручивается, чтобы дотянуться до элементов вне видимости. Реализация: в treeEl.dragover вычисляется зона 40px у краёв (getBoundingClientRect + e.clientY), направление пишется в _scrollDir (-1/0/+1), цикл requestAnimationFrame (_scrollRafId) крутит scrollTop на 8px/кадр. Остановка — _stopAutoScroll() в dragend (3 источника) и drop (rootZone/tree/grid). НЕ трогает _clearDragOver (иначе скролл прерывался при смене папки под курсором). scrollTop не меняет DOM → drag в WebView2 не срывается.
62. **Релиз 2.2.2-beta выпущен (08.06.2026).** Версия поднята в Cargo.toml/tauri.conf.json (2.2.2-beta) и extension/manifest.json (2.2.2). README.md и README.txt обновлены; раздел «ИСТОРИЯ ВЕРСИЙ» в README.txt переоформлен в формат + / ! и дополнен версиями 2.0/2.1/2.1.1. Архив URL-Album-2.2.2-beta.zip (URL-Album.exe + README.txt + extension\, без extension-keys) опубликован на GitHub, помечен Latest. В релиз вошли: фикс зоны дропа в корень (п.60) и автопрокрутка дерева (п.61).
63. **Feat (после релиза 2.2.2, для будущего релиза): имя базы в строке над деревом + версия в заголовке окна.** Строка #tree-root-drop (бывшая пустая зона дропа в корень) в покое показывает имя текущей базы (currentDbName), при drag — «↑ Корень», после drag возвращает имя базы (переключение через textContent в updateWindowTitle и в трёх dragstart/dragend; rootZone вынесен в глобал). CSS: opacity 0→1 (зона теперь всегда видима, layout не меняется — drag не затронут). Заголовок окна теперь «URL Album <версия>» без имени базы; добавлена const APP_VERSION в app.js (ВНИМАНИЕ: 4-е место, где надо обновлять версию при релизе, вместе с Cargo.toml/tauri.conf.json/manifest.json). Дубль имени базы из статусбара убран (в updateStatusLeft удалён parts.push с [currentDbName]) — теперь имя базы только над деревом.
64. **Fix (после релиза 2.2.2, для будущего релиза): ручная сортировка ссылок стрелками вверх/вниз не сохранялась между сессиями.** КОРНЕВАЯ ПРИЧИНА (неочевидная, искали долго): `get_tree` НЕ возвращал `sort_idx` во фронтенд — `struct TreeNode` (db.rs) не содержал поля `sort_idx`, и SELECT его не тянул (хотя `ORDER BY sort_idx,id` был). Поэтому на фронте у всех узлов `n.sort_idx === undefined`, и все сортировки (`.sort((a,b)=>(a.sort_idx??0)-...)`) сводились к сортировке по `id`. Стрелки писали `sort_idx` в БД относительно id-порядка, после перезапуска грид сортировался по `id` → «сброс». Дерево «работало» случайно (`allNodes` приходил `ORDER BY sort_idx,id`, стабильная сортировка V8 сохраняла порядок). Запись в БД при этом работала корректно (проверено в DB Browser). ФИКС: (1) db.rs — добавлено поле `pub sort_idx: i64` в `TreeNode` (после `visited`) и колонка `sort_idx` в SELECT `get_tree` (индекс 10, `count` сдвинут на 11 — индексы `row.get` строго по порядку колонок!); (2) app.js — `foldersFirst` в `buildTree` учитывает `sort_idx` внутри группы (папки по-прежнему выше ссылок); `loadFolderContents` сортирует закладки по `(sort_idx??0)-...||id`; `tbMoveItem`: siblings сортируется с tiebreaker `||a.id-b.id`, после перестановки обновляются И дерево (`saveOpenState→renderTree→restoreOpenState`, без `get_tree` т.к. `allNodes` уже мутирован) И грид (`await loadBookmarks`), восстанавливается выделение (`_activateTreeItem` + `gridSelectRow`). Коммит `a304912`.
65. **Feat (после релиза 2.2.2, для будущего релиза): перемещение ПАПОК стрелками вверх/вниз.** Расширен `tbMoveItem` (раньше работал только для bookmark): источник node теперь `activeBookmarkNode` (ссылка) ИЛИ `allNodes.find` по `activeFolderId` (папка, выбранная одиночным кликом в дереве — навигацию НЕ меняли). siblings фильтруются по `node.kind` (папки двигаются среди папок-сестёр, ссылки среди ссылок), `parent===null` корректно отбирает корневые папки. После перестановки: `renderTree` + `restoreOpenState` + `_activateTreeItem(node)` для обоих типов (серия нажатий работает); грид (`loadBookmarks`+`gridSelectRow`) обновляется только для ссылок (для папки `activeFolderId` не меняется, содержимое то же). Правка 2: `loadFolderContents` теперь сортирует и subfolders по `sort_idx` (раньше только bookmarks). Папки «выше ссылок» сохраняется через `foldersFirst`. Коммит `70a953b`.
66. **Feat (после релиза 2.2.2, для будущего релиза): drag&drop сортировка внутри папки в ГРИДЕ.** Ссылки и папки перетаскиваются для изменения порядка (раньше только стрелки). Зоны по clientY: ссылка→ссылка верх/низ = sort; папка→папка верх 25%/низ 25% = sort, середина 50% = вложить (move_node); ссылка→папка = вложить; папка→ссылка = игнор. Индикатор — box-shadow: inset (.sort-before/.sort-after), НЕ border (иначе reflow → срыв drag в WebView2). Общая логика sort_idx вынесена в `_applySortOrder(siblings, node)` — зовётся из стрелок (tbMoveItem) и из drag (_doSort); для папок _applySortOrder перезагружает грид через `loadFolderContents(node.parent)`. Заодно фикс: меню «Сортировка...» (sort_folder) не обновляло грид — allNodes sort_idx не синхронизировался с newOrder из Rust; добавлена синхронизация перед loadBookmarks. Коммит `8ed778a`. ДЕРЕВО drag-сортировку пока НЕ поддерживает (только грид).
