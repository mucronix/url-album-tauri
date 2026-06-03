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
- [ ] Drag & drop сортировка внутри папки (сейчас только кнопки вверх/вниз)
- [ ] `thumb` хранит абсолютный путь в DB → перейти на filename как у `favicon` (при переносе папки скриншоты ломаются)
- [ ] Browser import (`import_chromium`, `import_firefox`) → добавить `parent_id` (сейчас всегда в корень)
- [ ] Favicon: очистка orphaned файлов из `Data/favicons/` при удалении закладок
- [ ] Массовое выделение / batch operations
- [ ] Favicon: force refresh / TTL (YAGNI пока)

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

### Сессия 6 (2026-05-19) — Багфиксы
27. **Fix: favicon не появлялись после batch-загрузки** — `_finishFaviconBatch` теперь вызывает `renderTree()` + `loadFolderContents(activeFolderId)` после завершения. Ранее `updateFaviconInDOM` обновлял DOM, но WebView2 не перерисовывал без явного reload.
28. **Fix: скриншоты зависали на недоступных сайтах** — `spawn()` + poll `try_wait()` каждые 250мс вместо `status()`. Если deadline превышен — `child.kill()` принудительно, браузер всегда завершается в срок.
    - Дефолтный таймаут: 30с → **15с** (Настройки → Рисунок)
    - Rust fallback: `timeout.unwrap_or(15)`
    - JS fallbacks: `appSettings.thumbTimeout || 15`
29. **Release build** — `cargo build --release` → `dist/URL-Album-2.0-beta.zip` (3.7 MB). Полностью portable, в реестр ничего не пишется (только `reg query` для определения браузеров — read-only).
