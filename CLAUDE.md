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
- `#tree-root-drop` в потоке, управление через opacity (не absolute); причина reflow-срыва drag и эволюция решения — см. раздел Критические ловушки.
- Drop в корень всегда `parent = NULL` (эвристика `virtualRootId` удалена, п.69).
- `body.is-dragging` — класс на `<body>` при активном drag (dragstart/dragend всех трёх источников: tree-item, grid-row, grid-card). Управляет видимостью `#tree-root-drop` через CSS.
- Drag-сортировка: ссылки и папки в гриде, папки в дереве (порядок + вложение по зонам, п.66–67).

### Крупные задачи (отдельная сессия)

#### Архитектура хранения данных (решение принято, реализация отдельной сессией)

Каждая база — автономный контейнер: `ИмяБазы.db` + `ИмяБазы_Data/` (внутри `screenshots/` + `favicons/`).

- Папка данных = basename без `.db` + суффикс `_Data`. НЕ хранится в метаданных БД. Философия: «видно глазами в проводнике».
- Совместимость: при открытии искать `ИмяБазы_Data/`, если нет → фолбэк на `Data/` (старые базы). Без автопереименования, без миграции файлов, без диалогов (несколько баз могут делить одну `Data/` — переименовывать нельзя).
- Унифицировать: скриншоты в `screenshots/`, favicon в `favicons/` (сейчас рассинхрон — favicon в подпапке, скриншоты россыпью в `Data/`).
- Переименование базы — не решать сейчас (редкий сценарий). Когда появится «Файл → Переименовать базу» — там же переименовать папку.
- Затронет: `get_data_dir`, `do_screenshot`, `refresh_thumb`, HTTP-сервер, `faviconFilePath`, `thumbFilePath`, миграция `thumb`, создание базы, импорт.

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

- **`MAX_THUMB_CONCURRENCY = 1` — не поднимать.** При > 1 воркеры стартуют в одну секунду → одинаковый timestamp → один файл → браузеры перезаписывают друг друга → скриншоты путаются между ссылками. Имена теперь детерминированные `{id}.png` (перезапись без дублей), но ограничение `=1` остаётся.

- **Уникальный `--user-data-dir` per invocation** (`ua_screenshot_{id}`) — иначе конфликт профилей Edge/Chrome headless при параллельных вызовах.

- **Готовность скриншота = ПОЯВЛЕНИЕ ФАЙЛА, а не выход процесса браузера.** Headless Edge/Chrome-родитель может выйти ДО записи PNG дочерним процессом (на быстрых машинах — выход за ~260мс с файлом 0 КБ); плюс Windows Defender задерживает видимость только что созданного файла на **незнакомых путях** (репутационный скан новой/переименованной папки). Поэтому `do_screenshot` решает по `path.exists()` (записанный файл = успех, даже если браузер убит/вышел не чисто) + **ретрай ожидания файла с потолком = `t` секунд** (настройка «Время ожидания», НЕ зашитое число), плюс поллинг стабильного размера и дедлайн `t+8`. Имя **детерминированное `{id}.png`** (`remove_file` перед spawn → без дублей; cache-busting `?v=` в UI; авто-обновление detail-view в `_applyThumbToCard`). Симптом до фикса: на быстром ПК сохранялось 14/46, на медленном ноуте — всё ОК. Дефолт ожидания 12с. (Сессия 19, `a2309df`.)

### Импорт браузеров (Chromium профили)

- **`detect_browsers_list` сканирует ВСЕ профили в `User Data` (`read_dir`, по образцу Opera-скана), а не только `Default`.** Иначе Edge/Brave с не-Default профилем (`Profile 1`/`Profile 2` — типично при входе в аккаунт) не находятся для импорта, хотя «Обнаружить браузеры» (`detect_browser_exes`, по `.exe`) их видит. Ключ: **импорт ищет файл `Bookmarks` в профиле, обнаружение — `.exe`** → браузер может быть установлен без `Default\Bookmarks`. Каждый профиль с `Bookmarks` = отдельная запись **«Браузер — Профиль»**, id `{браузер}__{папка}` (стабилен — `import_from_browser` повторно зовёт `detect_browsers_list` и ищет по id). Формат у всех Chromium один (`import_chromium`); Firefox/Opera — свои сканы, не трогать. (Сессия 19, `499170c`.)

### Браузерное расширение

- **`POST /api/v1/handshake`, не GET.** Браузер не шлёт заголовок `Origin` на простые GET-запросы — Origin-проверка всегда давала 403. POST шлёт Origin.

- **`GET /api/v1/folders` — Origin-проверка ослаблена намеренно.** Браузер не шлёт Origin на GET без кастомных заголовков → нельзя требовать совпадение. Защита — токен `X-UA-Token`. Логика: Origin прислан и не совпадает → 403; пустой → пропустить.

- **Extension ID `imekfalcnffmmmabcjapmihbocjabecf` зафиксирован через `"key"` в `manifest.json`.** Приватный ключ `extension-keys/private.key.pem` **не в git** (`extension-keys*/` в `.gitignore`) — нужен для восстановления того же ID. Не потерять.

### Релиз

- **4 места для обновления версии:** `Cargo.toml`, `tauri.conf.json`, `extension/manifest.json` (без `-beta` — Chrome не поддерживает), `APP_VERSION` в `app.js`.

- **Ссылка в README — прямая** (`/releases/download/vX.Y.Z/...`), не `/releases/latest/download/...` — `latest` не видит pre-release, даёт 404.

- **ZIP без `extension-keys/`** — в `.gitignore`, в архив не попадает. Состав: `URL-Album.exe` + `README.txt` + `extension\`.

- **Релизный ZIP паковать ТОЛЬКО 7-Zip с `-mx=9`** (`& 'C:\Program Files\7-Zip\7z.exe' a -tzip -mx=9`), НЕ PowerShell `Compress-Archive` — последний сжимает вдвое хуже (7.6 MB против 3.7 MB при том же exe 8.11 MB).

### Поддерживаемые ОС

- **Минимум Windows 10.** WebView2 требует `ProcessPrng` из `bcryptprimitives.dll` — появилась в Windows 8+. На Win7 WebView2 (включая v109) падает с "ProcessPrng не найдена". DLL-шим отклонён — антивирусы флагают как малварь.

### JS — синхронизация дерева и грида

- **После изменения порядка/содержимого обновлять ОБА — дерево (`renderTree`) И грид (`loadFolderContents`/`loadBookmarks`) + синхронизировать `allNodes sort_idx`.** Рендерятся раздельно, легко забыть одно. Баг повторялся трижды: favicon batch не обновлял дерево (п.59), drag-сортировка папок не обновляла грид, `sort_folder` не обновлял грид (allNodes был stale). Паттерн: `allNodes[i].sort_idx = newIdx` → `renderTree` → `loadFolderContents`/`loadBookmarks`.
- **`dataDir` обновлять в `showApp` при каждом открытии базы, не только при старте.** Задаётся один раз в `init` → при `switch_db`/`open_db` остаётся от первой базы → картинки и favicon второй базы ищутся в `Data/` первой. Фикс: `dataDir = await invoke('get_data_dir')` в начале `showApp` перед `renderTree`.

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

> Полная история (сессии 1-18, релизы до 2.2.4-beta включительно) перенесена в HISTORY.md.