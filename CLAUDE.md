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
- `MAX_FAVICON_CONCURRENCY = 5` — константа в начале app.js (intentional rate limiting)
- `dataDir` — путь к Data/ (загружается при старте через `get_data_dir`)
- `faviconFilePath(filename)` — нормализует path separators для `convertFileSrc` на Windows
- `setFaviconOnEl(el, src)` — ставит favicon img, при ошибке восстанавливает `●`
- `extractDomain(url)` — извлечь домен для dedup
- `buildFaviconQueue(bookmarks)` — dedup по домену; один item на домен + `sameIds[]`
- `_runFaviconWorker()` — worker loop (5 параллельных invoke)
- `applyFaviconToDOM(item, filename)` — обновить allNodes + DOM + вызвать `update_node_favicon` для каждого sameId
- `updateFaviconInDOM(nodeId, filePath)` — live update tree + grid + detail
- `loadSingleFavicon(node)` — одна ссылка, без панели, после загрузки reload грида
- `startFaviconBatch(folderNode, recursive)` — запуск batch с прогресс-панелью
- Контекстное меню ссылки: "Загрузить favicon"
- Контекстное меню папки: "Загрузить favicon'ы" (recursive)

**Drag & Drop:**
- Все элементы дерева и grid-строки draggable
- Папки — drop targets (в дереве и в grid)
- Auto-expand при hover 650ms
- Валидация: no self-parent, no circular refs
- После drop: `get_tree` + re-render + reload panel

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
- [ ] `tbMoveItem` — работает только внутри одной папки
- [ ] Поиск — breadcrumb не всегда обновляется при клике на папку из результатов
- [ ] `backup_db` с `set_parent(&window)` — может вызывать DPI issues на Windows

### Архитектурные ограничения
- `rfd::AsyncFileDialog` без `set_parent` на Windows (убрано из `open_db` из-за DPI-бага)
- `allNodes` — полная перезагрузка при каждом изменении через `invoke('get_tree')`
- `thumb` хранит полный абсолютный путь в DB (legacy, в отличие от `favicon` который хранит только filename)

### Что НЕ сделано / очередь
- [x] Контекстное меню для папок в правой панели — реализовано
- [x] Proxy settings — вкладка удалена (системный WARP, незачем)
- [x] Импорт из другой базы — реализован (Перенос → Из другой базы...)
- [x] Настройка размера шрифта — реализована (ползунок 8–18px, `--ui-font` + calc)
- [x] **DnD в корень** — `move_node(Option<i64>)`, `#tree-root-drop` drop-зона (position: absolute, body.is-dragging), `virtualRootId` для legacy-обёртки
- [x] **Статусбар** — `#statusbar` (flex, 20px, `var(--bg2)`, снизу окна). Левая часть: `Папок: N · Ссылок: M · В папке: K · [name.db]`; при поиске — `Найдено: X`. Правая часть: временные сообщения (3с, `.sb-temp`) и sticky-прогресс (`.sb-sticky`, акцент). API: `setStatus(text, {sticky})`, `clearStatus()`, `updateStatusLeft()`. Интегрирован в: `showApp`, `showImportScreen`, `renderTree`, `loadFolderContents`, `renderSearchResults`, `clearSearchUI`, favicon/thumb batch, импорт из базы (замена alert). `currentDbName` устанавливается в `updateWindowTitle()`.
- [x] **Пункт "Файл → Выход" удалён** — `window.close()` в WebView2 уводил на `about:blank`, оставляя пустую рамку. Закрытие только через ×/Alt+F4. WAL-безопасность подтверждена: `PRAGMA synchronous=FULL` гарантирует fsync каждой записи в WAL; `sqlite3_close` при выходе автоматически делает финальный checkpoint (last-connection WAL merge — документированное поведение SQLite).
- [ ] Drag & drop сортировка внутри папки (сейчас только кнопки вверх/вниз)
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

### Сессия 1 (до 2026-05-15)
1. Multi-DB support — `db_path: Mutex<PathBuf>`, `switch_db()`, `last_db.txt`
2. Compact list view в правой панели вместо card grid
3. Resizable columns (`--col-name-w` CSS var, drag handler)
4. Resizable sidebar splitter (сохраняется в settings)
5. Synchronized dual-pane navigation (tree ↔ grid)
6. Drag & drop с `move_node` (circular ref validation в Rust)
7. `normalize_url()` — https:// авто-добавление без изменения БД
8. Tree toggle fix: `selectFolder(id, expand=false)` для tree-clicks
9. `CREATE_NO_WINDOW` — убрано мигание консоли при открытии ссылок

### Сессия 2 (2026-05-15–16)
10. **Favicon loading** — полная система: Rust async fetch + JS queue + domain dedup + progress panel
    - `fetch_favicon` (4 стратегии: favicon.ico → HTML → DuckDuckGo → Google), `get_data_dir`, `update_node_favicon`
    - `is_valid_image()` — magic bytes, SVG check (`<svg`/`<?xml`), отсеивает HTML
    - Cache validation — перезагружает битые кэш-файлы автоматически
    - Browser UA: Chrome 124 для обхода Cloudflare
    - `faviconFilePath()` — нормализация path separators на Windows
    - `sameIds` domain dedup — все ноды домена персистируются в DB
    - Favicon в дереве, гриде, detail view
11. **Tree UX** — полный рефакторинг поведения:
    - `[+]/[-]` кнопки через CSS `::before` (data-has-children)
    - Клик на `[+]/[-]` = только toggle; клик на label = только выделение+грид; dblclick = toggle
    - Серое выделение только на `.label` (не полная строка)
    - ↑↓ стрелки выбирают И активируют; клик фокусирует item
    - Папки всегда выше ссылок (`buildTree` сортирует по kind)
    - PNG иконки папок (`ui/icons/`) — pixel-art, CSS переключает по `.open`
12. **Grid single click** → `openDetailView` (карточка); double click → открыть в браузере
13. **`open_url`** → `rundll32.exe url.dll,FileProtocolHandler`; новая `open_file` для локальных файлов
14. **Контекстные меню** — убран "Проверить" из меню ссылки; упорядочены пункты; сортировка: один пункт + toggle asc/desc с ▲▼
15. **`refresh_thumb`** — принимает width/height/timeout из настроек; дефолт 1280×800, **15сек**; кнопка "По умолчанию"
16. **Окно** — `center: true`, `minWidth: 500` (Windows Snap корректно)
17. **Очистка** — удалены test screenshots, дубликаты иконок

### Сессия 4 (2026-05-19)
22. **Batch thumbnail refresh** — пакетное обновление скриншотов для папки:
    - Пункт "Обновить рисунки" в контекстном меню папки (после "Загрузить favicon'ы")
    - `#thumb-panel` — новая прогресс-панель (HTML + CSS), зеркало `#favicon-panel`, z-index: 501
    - `startThumbBatch(folderNode)` — только прямые ссылки папки (не рекурсивно)
    - `_runThumbWorker()` — `MAX_THUMB_CONCURRENCY = 1`, обновляет `allNodes` + DOM грида
    - `_applyThumbToCard(id, title, newPath)` — хелпер обновления карточки в гриде; используется и в `_runThumbWorker`, и в `refreshThumb`
    - `makeDlgDraggable` на `#tp-titlebar` — панель перетаскивается
    - **Fix:** `refresh_thumb` переведён из `fn` в `async fn` + `tauri::async_runtime::spawn_blocking` — `std::process::Command::status()` больше не блокирует IPC-поток и UI
    - **Fix:** уникальный `--user-data-dir` per invocation (`ua_screenshot_{id}`) — устранён конфликт при параллельных вызовах; temp dir удаляется после каждого скриншота
    - **Fix:** `makeDlgDraggable` сбрасывает `bottom`/`right` → `auto` при начале drag — панели с `bottom:` позиционированием не растягиваются
    - **Fix:** `#import-screen` скрыт по умолчанию — устранено мигание стартового экрана при Ctrl+R
    - **Cleanup:** `CREATE_NO_WINDOW` добавлен к browser Command в `spawn_blocking` (консистентно с остальным кодом)

### Сессия 3 (2026-05-17–18)
18. **Tree UX — доработки:**
    - `selectFolder(id, false, true)` — noTreeExpand=true: одиночный клик не трогает дерево вообще
    - `[+]/[-]` и dblclick НЕ вызывают `collapseSiblingBranches` — все папки независимы
    - При старте: все папки закрыты, первая только подсвечена
    - Правый клик по папке в дереве — подсвечивает папку (добавлен `.active`)
    - Правый клик по папке в гриде — `showFolderContextMenu` (было `return`)
19. **Меню "Вид"** — один toggle-пункт вместо двух:
    - `toggle-expand-all` в `CMD_REGISTRY`, `MENU_DATA`, `handleMenuAction`, `handleToolbarAction`
    - `_syncExpandToggleUI()` — синхронизирует текст+иконку меню и toolbar кнопки
    - Вызывается при открытии меню "Вид" и после каждого toggle
    - `expand-all` / `collapse-all` полностью удалены (из CMD_REGISTRY, handlers, DEFAULT_TOOLBAR)
20. **Меню закрывается** при `window.blur` (клик на titlebar, Alt+Tab)
21. **`group.dataset.id = menu.id`** — добавлен в buildMenubar для идентификации групп меню

### Сессия 5 (2026-05-19) — Реструктуризация меню
22. **Новая архитектура меню** — принцип "НАД ЧЕМ":
    - **Файл** = lifecycle БД: Создать.../Открыть.../Последние базы▶/Закрыть / Создать резервную копию.../с рисунками... / Свойства базы/Настройки/Выход
    - **Ссылки** = только операции над ссылками (без Import/Export/Backup/Sort)
    - **Перенос** = новое меню: Импорт▶ (браузер, другая база, HTML, TXT, sync, ua.dat) / Экспорт▶ / Браузеры
    - **Поиск** = только "Найти" (дубликаты перенесены в Ссылки)
    - **Вид** — без изменений
    - Пункт "Восстановить резервную копию" удалён (дублировал "Открыть базу")
23. **Новые Rust команды:**
    - `close_db` — checkpoint WAL, JS показывает welcome screen
    - `get_recent_dbs()` — список последних баз из `recent_dbs.txt` (max 10)
    - `get_db_properties()` → `DbProperties { path, size_bytes, folder_count, bookmark_count }`
    - `save_recent_db(path)` — вызывается из `switch_db` автоматически
    - Import команды (`import_html`, `import_txt_lines`, `import_sync`, `import_uadat_pick`, `import_txt_urls`, `db::import`) — добавлен `parent_id: Option<i64>` для импорта в конкретную папку
24. **Диалог "Свойства базы"** — показывает путь, размер, кол-во папок/ссылок; кнопка "Очистить базу"
    - `openDbPropertiesDialog()`, `formatBytes(bytes)`, `#dbprops-overlay`
    - `.win-btn-danger` / `.win-btn.win-btn-danger:not(:disabled):hover` — красная кнопка опасного действия
25. **Новое контекстное меню папки:**
    - Новая папка (`createFolderAndRename(folderNode.id)`) / Переименовать / Удалить
    - Импорт в папку▶ (`buildFolderImportSubmenu`) / Экспорт папки▶
    - Проверить ссылки / Обновить favicon'ы / Обновить рисунки
    - Сортировка▶ / Свойства
    - `invokeFolderImport(action, parentId)` — передаёт parentId в import команды
26. **Динамическое подменю "Последние базы"** — `_populateRecentDbs(drop)` вызывается при каждом открытии File меню
    - Элементы: `entry.dataset.recentDbs = '1'` для lookup
    - Click на запись: `invoke('switch_db', { newPath: p }).then(() => showApp())`

### Сессия 7 (2026-05-19) — Drag & Drop fix
30. **Fix: drag & drop не работал вообще** — Tauri по умолчанию перехватывает все OS-level drag-события для своего механизма drop файлов в окно (`dragDropEnabled: true`). Это блокировало `dragstart` на всех элементах. Фикс: `"dragDropEnabled": false` в `tauri.conf.json`.
31. **Refactor: DnD переписан на event delegation** — вместо handlers на каждом элементе, один `dragover`/`drop` на `treeEl` и `gridEl`. Используется `e.target.closest('.tree-item:not(.link)')` и `.closest('.card-folder')` для определения цели.
32. **Fix: убрана лишняя проверка "already there"** из `_isDragValid` — теперь перемещение в ту же папку просто делает no-op, но не блокирует drag visually.

### Сессия 8 (2026-05-21) — Тестирование Win7, документация
33. **Тестирование Windows 7** — установка VirtualBox 7.1.6 + VM Windows 7 Pro SP1 x64:
    - Guest Additions 7.1.x не устанавливались — `ERROR_AUTHENTICODE_TRUST_NOT_ESTABLISHED` (SHA-2 подпись, нужен KB3033929)
    - KB3033929 через ISO (создан через PowerShell IMAPI2FS) — не помог, bcdedit testsigning on — тоже
    - Решение: Guest Additions **6.1.48** (SHA-1 подпись) — установились без патчей
    - WebView2 bootstrapper: `ProcessPrng не найдена в bcryptprimitives.dll` — функция из Windows 8+
    - WebView2 v109 (139 МБ, Internet Archive) — та же ошибка
    - **Вывод: Windows 7/8 не поддерживается** — WebView2 требует Win8+ API (`ProcessPrng` в bcryptprimitives.dll). DLL-шим отклонён — антивирусы будут флагать как малварь.
34. **README и релиз обновлены** — убраны упоминания Windows 7/8 везде:
    - `README.md`: требования теперь **Windows 10 / 11** (64-bit)
    - `dist/URL-Album-2\README.txt`: то же
    - GitHub release: описание и ZIP обновлены
    - Минимальная поддерживаемая ОС: **Windows 10**

### Сессия 9 (2026-06-03) — Чистка, новая фича импорта
35. **Репозиторий** — проект переехал в `url-album-tauri` (чистая папка), новый GitHub-репо `skljar/url-album-tauri`, релиз v2.0-beta опубликован. Topics: bookmark-manager, tauri, rust, windows, sqlite, desktop-app.
36. **Удалена вкладка "Прокси"** из настроек — была заглушкой (proxy-поля сохранялись в settings.json но нигде не применялись). Системный WARP покрывает эту задачу. Удалено: HTML-блок `#stab-proxy`, таб, 5 полей в `appSettings`, `syncProxyFields`, populate/save код (~57 строк).
37. **Меню "Файл" перестроено** — плоский список вместо подменю "Резервная копия▶": `Создать резервную копию...` (backup_db) и `Создать резервную копию с рисунками...` (backup_db_with_data) вынесены на верхний уровень. Удалён дублирующий пункт "Восстановить резервную копию" (открывал тот же диалог что и "Открыть базу"). Мёртвый `case 'backup-restore'` удалён из handler.
38. **Фича "Импорт из другой базы"** (Перенос → Импорт → Из другой базы...):
    - **Rust:** `analyze_import_db` — read-only Connection к исходной БД, сравнение URL через `normalize_url_for_dedup` (убирает схему/www./trailing slash/регистр), возвращает `ImportAnalysis { source_path, total_bookmarks, new_count, duplicate_count, total_folders }`. `execute_import_db` — читает исходные ноды в память (spawn_blocking), затем BFS-обход needed_folders (только предки новых закладок), INSERT с перемаппингом old_id→new_id, INSERT новых закладок.
    - **UI:** диалог `#import-db-overlay` со статистикой; select назначения (корневые папки из allNodes + "Корень" + "Создать новую папку..."); при выборе "новая папка" — `invoke('create_folder')` → id → `execute_import_db`; alert с итогом + `refreshTree()`.
    - **Дедуп:** по нормализованному URL (не по домену) — `site.com/page1` и `site.com/page2` считаются разными.
    - **Поля Tauri → JS:** snake_case (source_path, new_count, etc.) — Tauri не конвертирует в camelCase на выходе.

39. **Настройка размера шрифта интерфейса** — ползунок 8–18px в Настройки → Общие:
    - **CSS:** переменная `--ui-font: 13px` на `:root`; все `font-size: 12px` → `calc(var(--ui-font) - 1px)`, `11px` → `-2px`, `10px` → `-3px`, `13px` → `var(--ui-font)`; нетронуты: 7–9px (иконки/стрелки), 14px, 16px (×), 20px
    - **JS:** `appSettings.uiFontSize = 13`; в `applySettings` — `document.documentElement.style.setProperty('--ui-font', size + 'px')`; ползунок с live-label в IIFE настроек
    - **Сохранение:** в `settings.json` через существующий `save_settings`; Rust не менялся

### Сессия 11 (2026-06-04) — Статусбар
41. **Статусбар** (`#statusbar`, HTML + CSS + JS):
    - **HTML:** `<div id="statusbar"><div id="sb-left"/><div id="sb-right"/></div>` — sibling к `#app` в `body` (body уже flex-column, никакой реструктуризации не потребовалось). Скрыт по умолчанию (`.hidden`).
    - **CSS:** `height: 20px`, `background: var(--bg2)`, `border-top: 1px solid var(--border)`, `font-size: calc(var(--ui-font) - 3px)`. `#sb-left` — flex:1, ellipsis. `#sb-right` — max-width:50%, ellipsis; `.sb-temp` (белый, 3с), `.sb-sticky` (акцент, пока не заменено).
    - **JS API:** `setStatus(text, {sticky=false})` — sticky остаётся до следующего вызова, temp гаснет через 3с. `clearStatus()` — сбросить. `updateStatusLeft()` — пересчитать `Папок/Ссылок/В папке|Найдено/[name.db]`.
    - **Интеграция:** `showApp()` (`await updateWindowTitle()` → `currentDbName`, показать бар), `showImportScreen()` (скрыть бар), `renderTree()` (updateStatusLeft), `loadFolderContents()` (`_sbInFolderCount`), `renderSearchResults()` (`_sbSearchCount`), `clearSearchUI()` (сброс `_sbSearchCount`). Favicon/thumb batch: sticky прогресс → temp "Готово". Импорт: `alert(...)` → `setStatus(...)` для успехов.
    - **Баг обнаружен (не исправлен):** `window.close()` в WebView2 уводит на `about:blank` вместо закрытия. Пункт меню "Выход" оставляет пустую рамку. → Исправлено в сессии 12.

### Сессия 12 (2026-06-04) — Удалён пункт "Выход"
42. **Удалён пункт "Файл → Выход"** — `window.close()` в WebView2 уводил на `about:blank`, оставляя пустую рамку окна. Проверена WAL-безопасность: `PRAGMA synchronous=FULL` + `sqlite3_close` при выходе = автоматический финальный checkpoint (документированное поведение SQLite для last-connection). `on_window_event(Destroyed)` в main.rs — no-op, данные сохраняются через Connection::drop. Удалены: строка `MENU_DATA` и `case 'quit':` обработчик.

### Сессия 13 (2026-06-04) — Фиксы пакетного обновления рисунков
43. **Имя файла скриншота: `{id}_{ts}.png` с миллисекундами** — было `{ts}.png` с `as_secs()`. При `MAX_THUMB_CONCURRENCY > 1` несколько воркеров стартуют в одну секунду → одинаковый `ts` → один и тот же файл → браузеры перезаписывают друг друга → скриншоты путаются между ссылками (видно визуально: чужие картинки на карточках). Фикс: `as_millis()` + `id` в имени гарантируют уникальность.
44. **`MAX_THUMB_CONCURRENCY = 1`** — было задокументировано в сессии 4, потом поднято до 3 и вернуло баг путаницы файлов. **Не поднимать без уникальных имён файлов.**
45. **`_finishThumbBatch` теперь вызывает `loadFolderContents` + обновляет открытый detail view** — раньше не вызывал (в отличие от `_finishFaviconBatch`). Результат: рисунки писались на диск и в DB, но UI обновлялся только после перезапуска программы.
46. **`.catch` в `_runThumbWorker` больше не глотает ошибки** — было `.catch(() => {})`, теперь выводит в статусбар. Раньше батч показывал "Готово N рисунков" даже при полном провале всех invoke.
47. **Удалена мёртвая иконка `quit`** — хвост от убранного в сессии 12 пункта "Выход".
    - **Диагностика в будущем:** F12/DevTools отключены в debug-сборке (WebView2 без `devtools: true` в конфиге). Для диагностики JS-ошибок — либо добавить `"devtools": true` в `tauri.conf.json` временно, либо выводить в UI (статусбар).
48. **Релиз v2.1.1-beta** — тег `v2.1.1-beta`, ZIP `URL-Album-2.1.1-beta.zip` прикреплён к GitHub Releases. Версия в `tauri.conf.json` и `Cargo.toml` поднята с `0.1.0` до `2.1.1`.
    - **`dist/` в `.gitignore` намеренно** — `README.txt` внутри `dist/` уходит в релиз через ZIP, в git его force-добавлять не нужно.
    - **Упаковка ZIP:** `Compress-Archive` в PowerShell, файлы кладутся в корень архива (без вложенной папки), exe переименовывается в `URL-Album.exe` перед упаковкой и удаляется из `dist/` после.

### Сессия 10 (2026-06-04) — DnD в корень
40. **DnD в корень** — перетаскивание узлов на верхний уровень дерева:
    - **Rust:** `move_node(id: i64, new_parent: Option<i64>)` — guards (`self-parent`, `circular ref`) только при `Some(np)`; `WHERE parent IS ?1` вместо `= ?1` (SQLite IS корректно работает с NULL); UPDATE работает с `Option<i64>` через rusqlite::params!.
    - **JS:** `virtualRootId` (let, обновляется в `buildTree`) — если в БД есть legacy-обёртка, drop в корень → обёртка; иначе → NULL. `_doDrop(null)` корректно проходит `_isDragValid` (null target). Guard `if (targetFolderId !== null)` в `_doDrop` — пропускает auto-expand для корня.
    - **UI:** `#tree-root-drop` (HTML) — `position: absolute; top:0; left:0; right:0; z-index:10; background: var(--bg2)`. Видим только при `body.is-dragging`. `body.is-dragging` выставляется в `dragstart` всех трёх источников (tree-item, grid-row, grid-card), снимается в `dragend`.
    - **`_clearDragOver()`** — расширен: чистит `.drag-over` с root-зоны. `dragend` у всех трёх источников рефакторнут: инлайновый cleanup → `_clearDragOver()`.
    - **Fix: reflow отменял drag из дерева** — `display: block` в нормальном потоке во время `dragstart` сдвигало tree-items, Chromium/WebView2 немедленно отменял drag (firing `dragend`). Фикс: `position: absolute` убирает зону из потока, layout не меняется, drag инициализируется нормально. DnD из грида не был затронут (источник в другой панели).

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

### Сессия 6 (2026-05-19) — Багфиксы
27. **Fix: favicon не появлялись после batch-загрузки** — `_finishFaviconBatch` теперь вызывает `renderTree()` + `loadFolderContents(activeFolderId)` после завершения. Ранее `updateFaviconInDOM` обновлял DOM, но WebView2 не перерисовывал без явного reload.
28. **Fix: скриншоты зависали на недоступных сайтах** — `spawn()` + poll `try_wait()` каждые 250мс вместо `status()`. Если deadline превышен — `child.kill()` принудительно, браузер всегда завершается в срок.
    - Дефолтный таймаут: 30с → **15с** (Настройки → Рисунок)
    - Rust fallback: `timeout.unwrap_or(15)`
    - JS fallbacks: `appSettings.thumbTimeout || 15`
29. **Release build** — `cargo build --release` → `dist/URL-Album-2.0-beta.zip` (3.7 MB). Полностью portable, в реестр ничего не пишется (только `reg query` для определения браузеров — read-only).
