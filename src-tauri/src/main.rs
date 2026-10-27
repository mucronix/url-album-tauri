#![windows_subsystem = "windows"]

#[macro_use]
mod i18n;
#[cfg(test)]
mod i18n_audit;
mod clipboard;
mod db;
mod importer;
mod logger;
mod relay;
mod shot_browser;

use std::sync::Mutex;
use rusqlite::Connection;
use tauri::{Manager, Emitter};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::image::Image;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct AppState {
    db:                 Mutex<Connection>,
    db_path:            Mutex<std::path::PathBuf>,
    pending_open:       Mutex<Option<(String, String)>>, // (url, title) from urlalbum:// scheme
    extension_add_mode: Mutex<String>,                   // "quick" | "dialog"
    user_hotkey:        Mutex<Option<String>>,           // пользовательский хоткей (помимо встроенного F8)
}


// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
fn get_tree(state: tauri::State<AppState>) -> Result<Vec<db::TreeNode>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    // Замер для работы над большими базами: здесь только SQL, сериализацию
    // и IPC покажет разница с замером того же вызова в JS
    let t = std::time::Instant::now();
    let tree = db::get_tree(&conn).map_err(|e| e.to_string())?;
    logger::log(&format!("Замер: get_tree — {} узлов, SQL {} мс", tree.len(), t.elapsed().as_millis()));
    Ok(tree)
}

/// Узлы по id для точечного обновления интерфейса без get_tree: тот же SELECT
/// и те же поля, удалённые не приходят.
#[tauri::command]
fn get_nodes(state: tauri::State<AppState>, ids: Vec<i64>) -> Result<Vec<db::TreeNode>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_nodes(&conn, &ids).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_bookmarks(
    state: tauri::State<AppState>,
    folder_id: i64,
) -> Result<Vec<db::Bookmark>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_bookmarks(&conn, folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn is_empty(state: tauri::State<AppState>) -> bool {
    state
        .db
        .lock()
        .map(|conn| db::is_empty(&conn))
        .unwrap_or(true)
}

/// Try to locate ua.dat or ua.dat.bak next to the executable.
#[tauri::command]
fn find_uadat() -> Option<String> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for name in ["ua.dat", "ua.dat.bak"] {
        let p = exe_dir.join(name);
        if p.exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

#[tauri::command]
fn import_uadat(state: tauri::State<AppState>, path: String) -> Result<usize, String> {
    let raw = std::fs::read(&path).map_err(|e| e.to_string())?;
    // The original file is Windows-1251 encoded
    let (text, _, _) = encoding_rs::WINDOWS_1251.decode(&raw);

    let nodes = importer::parse(&text);
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::import(&conn, &nodes, None).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_node(state: tauri::State<AppState>, id: i64, title: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("UPDATE nodes SET title = ?1 WHERE id = ?2", rusqlite::params![title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn set_folder_opener(state: tauri::State<AppState>, id: i64, opener: Option<String>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("UPDATE nodes SET opener = ?1 WHERE id = ?2", rusqlite::params![opener, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_folder(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    // Оба шага обязаны быть атомарными: отказ между ними пометил бы содержимое
    // удалённым, оставив саму папку в дереве — визуально пустая папка, и
    // содержимого нет даже в корзине.
    conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let res = (|| -> rusqlite::Result<()> {
        // (а) mark all descendants (excluding root) as deleted, keep their parent links intact
        conn.execute(
            "WITH RECURSIVE sub(id) AS (
                 VALUES(?1)
                 UNION ALL
                 SELECT n.id FROM nodes n JOIN sub s ON n.parent = s.id
             )
             UPDATE nodes SET deleted=1 WHERE id IN (SELECT id FROM sub) AND id != ?1",
            rusqlite::params![id],
        )?;
        // (б) detach root from tree and mark deleted, saving its original parent for restore
        conn.execute(
            "UPDATE nodes SET deleted=1, deleted_parent=parent, parent=NULL WHERE id=?1",
            rusqlite::params![id],
        )?;
        Ok(())
    })();

    match res {
        Ok(()) => {
            conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
            Ok(())
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK").ok();
            logger::log(&format!("удаление папки id={id} отменено, изменений нет: {e}"));
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn get_trash(state: tauri::State<AppState>) -> Result<Vec<db::TreeNode>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_trash(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn restore_node(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let (kind, deleted_parent): (String, Option<i64>) = conn.query_row(
        "SELECT kind, deleted_parent FROM nodes WHERE id=?1",
        rusqlite::params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).map_err(|e| e.to_string())?;

    let target_parent: Option<i64> = match deleted_parent {
        Some(dp) => {
            let ok: bool = conn.query_row(
                "SELECT COUNT(*) FROM nodes WHERE id=?1 AND (deleted IS NULL OR deleted=0)",
                rusqlite::params![dp],
                |r| r.get::<_, i64>(0),
            ).unwrap_or(0) > 0;
            if ok { Some(dp) } else { None }
        }
        None => None,
    };

    if kind == "folder" {
        // Restore all deleted descendants, keep their parent links intact
        conn.execute(
            "WITH RECURSIVE sub(id) AS (
                 VALUES(?1)
                 UNION ALL
                 SELECT n.id FROM nodes n JOIN sub s ON n.parent = s.id WHERE n.deleted = 1
             )
             UPDATE nodes SET deleted=0, deleted_parent=NULL
             WHERE id IN (SELECT id FROM sub) AND id != ?1",
            rusqlite::params![id],
        ).map_err(|e| e.to_string())?;
    }
    // Restore root or bookmark: place back at target_parent
    conn.execute(
        "UPDATE nodes SET deleted=0, parent=?1, deleted_parent=NULL WHERE id=?2",
        rusqlite::params![target_parent, id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn empty_trash(state: tauri::State<AppState>) -> Result<(), String> {
    let data_dir = {
        let p = state.db_path.lock().map_err(|e| e.to_string())?;
        p.parent().ok_or("no parent dir")?.to_path_buf().join("Data")
    };

    let (thumbs_to_delete, favicons_to_delete) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT thumb, favicon FROM nodes WHERE deleted=1"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<(Option<String>, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let thumbs: Vec<String> = rows.iter().filter_map(|(t, _)| t.clone()).collect();

        // Файл favicon удаляем только если на него не ссылается активный узел.
        //
        // Здесь проверка по активным ВЕРНА, хотя точно такая же в purge_node
        // оказалась багом: эта команда стирает все строки с deleted=1 разом,
        // поэтому «все, кроме стираемых» и «активные» — одно и то же множество.
        // В purge_node стирается лишь часть корзины, и там пришлось спрашивать
        // «id NOT IN (стираемые)». Не унифицировать два места по внешнему
        // сходству: один и тот же запрос отвечает здесь на другой вопрос.
        let favicons: Vec<String> = rows.iter()
            .filter_map(|(_, f)| f.clone())
            .filter(|fav| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM nodes WHERE favicon=?1 AND (deleted IS NULL OR deleted=0)",
                    rusqlite::params![fav],
                    |r| r.get(0),
                ).unwrap_or(1);
                count == 0
            })
            .collect();

        conn.execute("DELETE FROM nodes WHERE deleted=1", [])
            .map_err(|e| e.to_string())?;

        (thumbs, favicons)
    };

    for thumb in thumbs_to_delete {
        let _ = std::fs::remove_file(data_dir.join(&thumb));
    }
    for fav in favicons_to_delete {
        let _ = std::fs::remove_file(data_dir.join("favicons").join(&fav));
    }

    Ok(())
}

#[tauri::command]
fn purge_node(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let data_dir = {
        let p = state.db_path.lock().map_err(|e| e.to_string())?;
        p.parent().ok_or("no parent dir")?.to_path_buf().join("Data")
    };

    let (thumbs_to_delete, favicons_to_delete) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;

        // Determine ids to purge: node + deleted subtree if folder
        let kind: String = conn.query_row(
            "SELECT kind FROM nodes WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        ).map_err(|e| e.to_string())?;

        let ids: Vec<i64> = if kind == "folder" {
            let mut stmt = conn.prepare(
                "WITH RECURSIVE sub(id) AS (
                     VALUES(?1)
                     UNION ALL
                     SELECT n.id FROM nodes n JOIN sub s ON n.parent = s.id WHERE n.deleted = 1
                 )
                 SELECT id FROM sub"
            ).map_err(|e| e.to_string())?;
            // Bind to named variable so borrow lifetime is explicit to the compiler
            let mapped = stmt.query_map(rusqlite::params![id], |r| r.get::<_, i64>(0))
                .map_err(|e| e.to_string())?;
            mapped.filter_map(|r| r.ok()).collect()
        } else {
            vec![id]
        };

        // Collect thumb/favicon for all ids (Vec<i64> — safe for interpolation)
        let placeholders = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(
            &format!("SELECT thumb, favicon FROM nodes WHERE id IN ({})", placeholders)
        ).map_err(|e| e.to_string())?;
        let rows: Vec<(Option<String>, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let thumbs: Vec<String> = rows.iter().filter_map(|(t, _)| t.clone()).collect();

        // Favicon общий на домен, поэтому вопрос ровно один: останется ли после
        // нашего удаления хоть одна ссылка на файл. Считаем ВСЕ узлы, кроме
        // стираемых этим вызовом, а не только активные.
        //
        // Проверка по активным была багом: две закладки одного домена лежат в
        // корзине, «Удалить навсегда» одну — общий файл уходил, и у второй после
        // восстановления пропадал значок. Перечислять состояния («активные плюс
        // оставшиеся в корзине») не нужно: одно из них рано или поздно забудется,
        // а `id NOT IN (…)` формулирует исходный вопрос буквально.
        //
        // Запрос выполняется ДО DELETE ниже, поэтому исключение по id обязательно.
        // Список ids — i64, прочитанные из базы: интерполяция безопасна, это тот
        // же `placeholders`, которым удаляются строки. Для папки в него входит
        // вся удалённая ветка, поэтому дочерние ссылки не сойдут за «оставшиеся».
        let favicons: Vec<String> = rows.iter()
            .filter_map(|(_, f)| f.clone())
            .filter(|fav| {
                let count: i64 = conn.query_row(
                    &format!("SELECT COUNT(*) FROM nodes WHERE favicon=?1 AND id NOT IN ({})", placeholders),
                    rusqlite::params![fav],
                    |r| r.get(0),
                ).unwrap_or(1);   // сбой запроса → считаем, что файл ещё нужен
                count == 0
            })
            .collect();

        conn.execute(
            &format!("DELETE FROM nodes WHERE id IN ({})", placeholders), []
        ).map_err(|e| e.to_string())?;

        (thumbs, favicons)
    };

    for thumb in thumbs_to_delete {
        let _ = std::fs::remove_file(data_dir.join(&thumb));
    }
    for fav in favicons_to_delete {
        let _ = std::fs::remove_file(data_dir.join("favicons").join(&fav));
    }

    Ok(())
}

#[tauri::command]
fn clear_thumb(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let old_thumb: Option<String> = conn.query_row(
        "SELECT thumb FROM nodes WHERE id=?1", rusqlite::params![id], |r| r.get(0)
    ).ok().flatten();
    conn.execute("UPDATE nodes SET thumb = NULL WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    if let Some(old) = old_thumb {
        let data_dir = state.db_path.lock().map_err(|e| e.to_string())?
            .parent().ok_or("no parent dir")?.to_path_buf().join("Data");
        let _ = std::fs::remove_file(data_dir.join(&old));
    }
    Ok(())
}

#[tauri::command]
fn update_node_favicon(
    state: tauri::State<AppState>,
    id: i64,
    filename: String,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE nodes SET favicon = ?1 WHERE id = ?2",
        rusqlite::params![filename, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_data_dir(state: tauri::State<AppState>) -> Result<String, String> {
    let dir = state.db_path.lock().map_err(|e| e.to_string())?
        .parent().ok_or("no parent dir")?.to_path_buf().join("Data");
    Ok(dir.to_string_lossy().into_owned())
}

/// Имя файла снимка годится, только если это имя внутри `Data`, без пути:
/// `..`, разделители и `:` (диск, `C:x`, потоки NTFS) не пропускаются.
fn thumb_file_name_ok(name: &str) -> bool {
    !name.is_empty() && name != "." && !name.contains("..")
        && !name.contains(['/', '\\', ':'])
}

/// Время изменения файла снимка, миллисекунды Unix — дата рисунка в строке
/// состояния (в местное время переводит JS). `None` — имя с путём, файла нет
/// или время не читается.
#[tauri::command]
fn get_thumb_time(filename: String, state: tauri::State<AppState>) -> Option<i64> {
    if !thumb_file_name_ok(&filename) { return None; }
    let dir = std::path::PathBuf::from(get_data_dir(state).ok()?);
    let modified = std::fs::metadata(dir.join(&filename)).ok()?.modified().ok()?;
    let ms = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    i64::try_from(ms).ok()
}

/// Текст из буфера обмена для «Вставить» в меню полей ввода (см. clipboard.rs:
/// navigator.clipboard.readText в WebView2 показывает запрос разрешения).
/// `null` — текста в буфере нет; ошибка — буфер держит другая программа.
/// Блокирующая (паузы между попытками открыть буфер) — поэтому spawn_blocking.
#[tauri::command]
async fn clipboard_text() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(clipboard::read_text)
        .await.map_err(|e| e.to_string())?
}

// ── HTTP client / прокси ─────────────────────────────────────────────────────

/// URL для проверки прокси. Тот же хост, что и в фолбэке favicon: раз он
/// доступен там, где программа работает, годится и как пробный запрос.
const PROXY_TEST_URL: &str = "https://icons.duckduckgo.com/ip3/example.com.ico";

/// Срезать схему и хвостовой слэш у адреса прокси: пользователь привычно
/// пишет `http://10.0.0.1/`, а голый хост нужен и `reqwest::Proxy`, и флагу
/// `--proxy-server`. Общая точка для `build_proxy` и `proxy_server_arg`.
pub(crate) fn strip_proxy_scheme(host: &str) -> &str {
    let h = host.trim();
    let lower = h.to_ascii_lowercase();
    let h = if lower.starts_with("http://")       { &h[7..] }
            else if lower.starts_with("https://") { &h[8..] }
            else                                  { h };
    h.trim_end_matches('/')
}

/// Собрать http-прокси из уже разобранных параметров. Общая точка для
/// настроек (`proxy_from_settings`) и проверки из диалога (`test_proxy`) —
/// формат URL и правило про basic-auth обязаны совпадать в обоих путях,
/// иначе «Проверить» покажет успех для конфигурации, которой нет в бою.
///
/// Схему у хоста срезаем (`strip_proxy_scheme`): иначе из привычного
/// `http://10.0.0.1` вышло бы `http://http://10.0.0.1:8080`. Сам прокси
/// всегда `http://`: https-прокси и SOCKS в этой версии не поддержаны.
fn build_proxy(host: &str, port: u16, user: &str, pass: &str) -> Result<reqwest::Proxy, reqwest::Error> {
    let h = strip_proxy_scheme(host);
    let proxy = reqwest::Proxy::all(format!("http://{h}:{port}"))?;
    if user.is_empty() { Ok(proxy) } else { Ok(proxy.basic_auth(user, pass)) }
}

/// Разобранные настройки прокси. Общая точка чтения для reqwest-клиента и для
/// командной строки браузера — иначе разбор camelCase-ключей и порта
/// «числом или строкой» пришлось бы держать в двух копиях.
pub(crate) struct ProxyCfg {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) user: String,
    pub(crate) pass: String,
}

/// Прокси из `settings.json` (файл пишет JS, ключи camelCase).
/// Любая проблема — нет файла, кривой JSON, выключено, мусор в полях — даёт
/// `None`: при плохих настройках сеть должна работать напрямую, а не падать.
pub(crate) fn proxy_cfg_from_settings() -> Option<ProxyCfg> {
    let raw = read_settings_raw();
    if raw.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;

    if v.get("proxyEnabled").and_then(|x| x.as_bool()) != Some(true) {
        return None;
    }

    let host = v.get("proxyHost").and_then(|x| x.as_str()).unwrap_or("").trim();
    if host.is_empty() {
        return None;
    }

    // Порт JS может записать и числом, и строкой (<input type="number">) — берём оба.
    let port = match v.get("proxyPort") {
        Some(serde_json::Value::Number(n)) => n.as_u64()?,
        Some(serde_json::Value::String(s)) => s.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    if !(1..=65535).contains(&port) {
        return None;
    }

    Some(ProxyCfg {
        host: host.to_string(),
        port: port as u16,                              // порт уже проверен диапазоном 1..=65535
        user: v.get("proxyUser").and_then(|x| x.as_str()).unwrap_or("").trim().to_string(),
        pass: v.get("proxyPass").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    })
}

/// Прокси для reqwest (`fetch_favicon`, `check_url`). Только `http://`-прокси;
/// https-прокси и SOCKS в этой версии не поддержаны.
fn proxy_from_settings() -> Option<reqwest::Proxy> {
    let c = proxy_cfg_from_settings()?;
    build_proxy(&c.host, c.port, &c.user, &c.pass).ok()
}

/// Прокси для командной строки headless-браузера (скриншоты): `host:port`,
/// без схемы.
///
/// Прокси без авторизации отдаём браузеру напрямую. Для прокси с логином
/// возвращаем адрес локального релея (`relay::ensure_relay`): Chromium
/// игнорирует учётные данные в `--proxy-server` — на 407 он показал бы диалог
/// логина, которого в headless нет, и завершился бы молча, без PNG. Релей
/// слушает 127.0.0.1 без авторизации и сам подставляет `Proxy-Authorization`
/// при походе на внешний прокси.
///
/// `None` — прокси выключен, настроен криво или релей не поднялся: в этом
/// случае скриншот делается напрямую, без прокси.
fn proxy_server_arg() -> Option<String> {
    let c = proxy_cfg_from_settings()?;
    if c.user.is_empty() {
        return Some(format!("{}:{}", strip_proxy_scheme(&c.host), c.port));
    }
    let port = relay::ensure_relay()?;
    Some(format!("127.0.0.1:{port}"))
}

/// Единая точка сборки HTTP-клиента: таймаут, User-Agent и — если прокси
/// включён и корректно задан — маршрут через него. Без прокси поведение
/// в точности прежнее.
fn http_client(timeout: std::time::Duration, ua: &str) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder().timeout(timeout).user_agent(ua);
    if let Some(p) = proxy_from_settings() {
        b = b.proxy(p);
    }
    b.build().map_err(|e| e.to_string())
}

/// Вся цепочка текстов ошибки. Нужна, чтобы разглядеть «407» внутри ошибки
/// CONNECT-туннеля: при https через прокси 407 не доходит до `r.status()`,
/// а приходит ошибкой соединения — reqwest прячет суть во вложенной причине.
fn err_chain_text(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(c) = src {
        s.push_str("; ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}

/// Короткая причина для показа человеку: самая глубокая ошибка в цепочке
/// информативнее верхнего «error sending request for url (...)».
fn short_err(e: &reqwest::Error) -> String {
    let mut deepest = e.to_string();
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(c) = src {
        deepest = c.to_string();
        src = c.source();
    }
    if deepest.chars().count() > 120 {
        deepest = deepest.chars().take(117).collect::<String>() + "...";
    }
    deepest
}

/// Отказ авторизации на прокси. reqwest/hyper при CONNECT-туннеле пишет причину
/// СЛОВАМИ, без кода статуса. Реальная цепочка (сессия 20, прокси с --basic-auth,
/// пустой логин): «error sending request for url (...); client error (Connect);
/// tunnel error: proxy authorization required», флаги connect=true, status=None.
/// Отсюда две текстовые формулировки; «407» оставлен на случай прокси, который
/// отвечает настоящим статусом (http-запрос без туннеля).
fn is_proxy_auth_error(e: &reqwest::Error) -> bool {
    let t = err_chain_text(e).to_lowercase();
    t.contains("proxy authorization required")
        || t.contains("proxy authentication required")
        || t.contains("407")
}

/// Проверка прокси по параметрам ИЗ ПОЛЕЙ ДИАЛОГА, а не из `settings.json`:
/// пользователь проверяет то, что сейчас введено, ещё до сохранения.
#[tauri::command]
async fn test_proxy(host: String, port: u16, user: String, pass: String) -> Result<String, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err(t!("settings.proxy.no_host"));
    }
    if port == 0 {
        return Err(t!("settings.proxy.bad_port"));
    }
    let user = user.trim();

    let proxy = build_proxy(host, port, user, &pass)
        .map_err(|_| t!("proxy.bad_address"))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent("Mozilla/5.0 URL-Album-Checker/1.0")
        .proxy(proxy)
        .build()
        .map_err(|_| t!("proxy.bad_address"))?;

    match client.get(PROXY_TEST_URL).send().await {
        Ok(r) if r.status().as_u16() == 407 =>
            Err(t!("proxy.auth")),
        Ok(r) if r.status().is_success() =>
            Ok(t!("proxy.works", code = r.status().as_u16())),
        Ok(r) => Err(t!("proxy.http_error", code = r.status().as_u16())),
        Err(e) if e.is_timeout() =>
            Err(t!("proxy.timeout")),
        // Обязательно ВЫШЕ is_connect: у отказа авторизации connect=true,
        // и нижняя ветка перехватила бы случай раньше (проверено, сессия 20).
        Err(e) if is_proxy_auth_error(&e) =>
            Err(t!("proxy.auth")),
        Err(e) if e.is_connect() =>
            Err(t!("proxy.no_answer")),
        Err(e) => Err(t!("common.error", e = short_err(&e))),
    }
}

#[tauri::command]
async fn fetch_favicon(
    state: tauri::State<'_, AppState>,
    id: i64,
    url: String,
) -> Result<Option<String>, String> {
    // ── 1. Extract domain ────────────────────────────────────────────────
    let normalized = normalize_url(&url);
    let domain = match extract_domain(&normalized) {
        Some(d) if !d.is_empty() => d,
        _ => return Ok(None),
    };
    let safe = sanitize_domain(&domain);

    // ── 2. Build favicons dir (lock db_path briefly, then release) ────────
    let favicons_dir = {
        let p = state.db_path.lock().map_err(|e| e.to_string())?;
        p.parent().ok_or("no parent dir")?.to_path_buf()
            .join("Data").join("favicons")
    };
    std::fs::create_dir_all(&favicons_dir).map_err(|e| e.to_string())?;

    // ── 3. Cache hit: scan for {safe_domain}.* ────────────────────────────
    if let Ok(entries) = std::fs::read_dir(&favicons_dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            let stem  = std::path::Path::new(&fname)
                .file_stem().unwrap_or_default()
                .to_string_lossy().to_string();
            if stem == safe {
                // Validate cached file — delete and re-fetch if corrupted
                let file_path = favicons_dir.join(&fname);
                if let Ok(cached) = std::fs::read(&file_path) {
                    if is_valid_image(&cached) {
                        let conn = state.db.lock().map_err(|e| e.to_string())?;
                        conn.execute(
                            "UPDATE nodes SET favicon = ?1 WHERE id = ?2",
                            rusqlite::params![fname, id],
                        ).map_err(|e| e.to_string())?;
                        return Ok(Some(fname));
                    } else {
                        // Stale/corrupt cache — delete and re-fetch
                        let _ = std::fs::remove_file(&file_path);
                    }
                }
            }
        }
    }

    // ── 4. HTTP client ───────────────────────────────────────────────────
    let client = http_client(
        std::time::Duration::from_secs(5),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    )?;

    let favicon_ico = format!("https://{}/favicon.ico", domain);

    // Шаги 5-8 под общим лимитом 12с: мёртвый URL сдаётся за ≤12с (вместо ~40с),
    // но DuckDuckGo (быстрый фолбэк) успевает дёрнуться даже после двух медленных первых шагов.
    let chain = async {
    let mut strategy = "";   // какая из четырёх попыток дала картинку — для журнала
    // ── 5. Attempt favicon.ico ────────────────────────────────────────────
    let (raw_bytes, ext) = match client.get(&favicon_ico).send().await {
        Ok(resp) if resp.status().is_success() => {
            let ct = resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let ext = ext_from_content_type(&ct);
            match resp.bytes().await {
                Ok(b) if is_valid_image(&b) => (Some(b), ext),
                _ => (None, "ico"),
            }
        }
        _ => (None, "ico"),
    };

    if raw_bytes.is_some() { strategy = "favicon.ico"; }

    // ── 6. Fallback: parse HTML <head> for <link rel="icon"> ─────────────
    let (raw_bytes, ext) = if raw_bytes.is_none() {
        let page = format!("https://{}/", domain);
        let base = format!("https://{}", domain);
        match client.get(&page).send().await {
            Ok(resp) => match resp.text().await {
                Ok(html) => match find_icon_href(&html, &base) {
                    Some(icon_url) => match client.get(&icon_url).send().await {
                        Ok(r2) if r2.status().is_success() => {
                            let ct = r2.headers()
                                .get("content-type")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                            let ext = ext_from_content_type(&ct);
                            match r2.bytes().await {
                                Ok(b) if is_valid_image(&b) => (Some(b), ext),
                                _ => (None, "ico"),
                            }
                        }
                        _ => (None, "ico"),
                    },
                    None => (None, "ico"),
                },
                _ => (None, "ico"),
            },
            _ => (None, "ico"),
        }
    } else {
        (raw_bytes, ext)
    };

    if strategy.is_empty() && raw_bytes.is_some() { strategy = "<link rel=icon>"; }

    // ── 7. Fallback: DuckDuckGo favicon service (handles Cloudflare-protected sites) ──
    let (raw_bytes, ext) = if raw_bytes.is_none() {
        let ddg_url = format!("https://icons.duckduckgo.com/ip3/{}.ico", domain);
        match client.get(&ddg_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let ct = resp.headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let ext = ext_from_content_type(&ct);
                match resp.bytes().await {
                    Ok(b) if is_valid_image(&b) => (Some(b), ext),
                    _ => (None, "ico"),
                }
            }
            _ => (None, "ico"),
        }
    } else {
        (raw_bytes, ext)
    };

    if strategy.is_empty() && raw_bytes.is_some() { strategy = "DuckDuckGo"; }

    // ── 8. Fallback: Google favicon service ──────────────────────────────
    let (raw_bytes, ext) = if raw_bytes.is_none() {
        let g_url = format!("https://www.google.com/s2/favicons?domain={}&sz=32", domain);
        match client.get(&g_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let ct = resp.headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let ext = ext_from_content_type(&ct);
                match resp.bytes().await {
                    Ok(b) if is_valid_image(&b) && b.len() > 68 => (Some(b), ext),
                    _ => (None, "ico"),
                }
            }
            _ => (None, "ico"),
        }
    } else {
        (raw_bytes, ext)
    };

        if strategy.is_empty() && raw_bytes.is_some() { strategy = "Google"; }

        (raw_bytes, ext, strategy)
    };
    let (raw_bytes, ext, strategy) = match tokio::time::timeout(std::time::Duration::from_secs(12), chain).await {
        Ok(v)  => v,
        Err(_) => {
            logger::log(&format!("favicon {domain}: истёк общий лимит 12с"));
            return Ok(None);         // «нет фавикона», без паники
        }
    };

    // ── 9. Nothing found ─────────────────────────────────────────────────
    let bytes = match raw_bytes {
        Some(b) => b,
        None => {
            logger::log(&format!("favicon {domain}: все четыре стратегии не дали результата"));
            return Ok(None);
        }
    };
    logger::log(&format!("favicon {domain}: получен через {strategy}"));

    // ── 10. Save file + update DB ────────────────────────────────────────
    let filename  = format!("{}.{}", safe, ext);
    let file_path = favicons_dir.join(&filename);
    std::fs::write(&file_path, &bytes).map_err(|e| e.to_string())?;

    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE nodes SET favicon = ?1 WHERE id = ?2",
            rusqlite::params![filename, id],
        ).map_err(|e| e.to_string())?;
    }

    Ok(Some(filename))
}

// ── Снимок: размеры и ожидание из настроек ───────────────────────────────────

/// Значения по умолчанию для снимка. Единственное место в Rust, где эти числа
/// живут: их берут и `thumb_cfg_from_settings`, и фолбэк в `do_screenshot`.
/// Сюда же придёт будущая общая валидация (рамки полей диалога — 320–3840 /
/// 240–2160 / 5–120): рамки и дефолты обязаны лежать рядом, иначе разъедутся.
const THUMB_W_DEFAULT: u32 = 1280;
const THUMB_H_DEFAULT: u32 = 800;
const THUMB_TIMEOUT_DEFAULT: u32 = 12;

/// Размер снимка и ожидание из `settings.json` (файл пишет JS, ключи camelCase).
/// Сделано по образцу `proxy_cfg_from_settings`: файл читается при КАЖДОМ
/// вызове — иначе настройка подхватывалась бы только после перезапуска.
///
/// Зачем: `do_screenshot` зовут два пути. Из `refresh_thumb` размеры приходят
/// от фронтенда, а HTTP-сервер расширения в режиме «быстрое добавление» идёт
/// мимо интерфейса и передаёт `None` — и снимок выходил 1280×800 при любой
/// настройке. Проверено вживую: с галочкой «Открывать окно правки» размер
/// верный, без неё — всегда дефолтный.
///
/// Рамок (min/max) здесь намеренно НЕТ: JS сохраняет настройку через
/// `parseInt(...) || 1280`, тоже без рамок, и ограничение только здесь дало бы
/// расхождение того же класса, который эта правка чинит — один и тот же
/// `settings.json` давал бы разный размер в разных путях. Валидация нужна
/// общая для обоих путей, отдельной задачей.
fn thumb_cfg_from_settings() -> (u32, u32, u32) {
    // Нет файла, BOM, испорченный JSON — дефолты и не падаем: снимок важнее
    // настройки размера. `read_settings_raw` отдаёт пустую строку, она тоже
    // не разбирается и приходит сюда же.
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&read_settings_raw()) else {
        return (THUMB_W_DEFAULT, THUMB_H_DEFAULT, THUMB_TIMEOUT_DEFAULT);
    };
    (
        thumb_u32(v.get("thumbWidth"),   THUMB_W_DEFAULT),
        thumb_u32(v.get("thumbHeight"),  THUMB_H_DEFAULT),
        thumb_u32(v.get("thumbTimeout"), THUMB_TIMEOUT_DEFAULT),
    )
}

/// Одно число из настроек. `<input type="number">` пишет и числом, и строкой —
/// принимаем оба, как с портом прокси. Ноль, отрицательное, нечисловая строка,
/// отсутствующий ключ — это отсутствие значения, а не «снимок 0×0»: дефолт.
///
/// Известная граница разбора, лечится вместе с общей валидацией: дробное
/// значение (`1920.0`) `as_u64()` не отдаёт вовсе — уйдём на дефолт, то есть
/// получим ровно тот симптом, который эта правка чинит. JS так не пишет
/// (`parseInt`), случай возможен только при правке `settings.json` руками.
fn thumb_u32(v: Option<&serde_json::Value>, default: u32) -> u32 {
    let n = match v {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    };
    if n == 0 || n > u32::MAX as u64 { default } else { n as u32 }
}

/// Итог снимка: имя файла и чем снято. `fallback` — Some, если снял не
/// назначенный браузер (папки или по умолчанию), а запасной; там причина.
/// Уходит в JS как есть (`refresh_thumb`): поля snake_case, `fallback` — `null`.
#[derive(serde::Serialize)]
pub(crate) struct Shot {
    pub filename: String,
    pub browser:  String,
    pub fallback: Option<String>,
}

/// Срок проверки адреса до запуска браузера (DNS + TCP вместе).
const PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// Ответ на второй снимок той же ссылки — код, не текст: текст для человека
/// держит app.js. Тот же код — `SHOT_BUSY` в app.js: по нему пакет считает
/// ссылку «пропущено», а не «не удалось».
pub(crate) const SHOT_BUSY: &str = "shot_busy";

/// Файлы снимков, которые сейчас снимаются. Вектор, а не `HashSet`: снимков
/// разом единицы, а `Vec::new` годится для `static` без ленивой инициализации.
static SHOTS_RUNNING: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Право снимать в `Data\{id}.png`; отпускается в `Drop`.
///
/// Без него два снимка одной ссылки (пакет, «Обновить рисунок», окно «Новая
/// ссылка», расширение в режиме «быстро» — последнее мимо JS) делили один
/// файл: каждый делает `remove_file` и ждёт появления того же пути, так что
/// мог стереть чужой результат или принять чужой файл за свой (id=193 в госте
/// Win7 «снят за 11053мс (--headless=new)», хотя Opera 95 так не снимает).
/// Ключ — путь к файлу, а не id: две базы в одной папке делят `Data`, а
/// одинаковые id в разных `Data` друг другу не мешают.
struct ShotLock(std::path::PathBuf);

impl ShotLock {
    /// `None` — этот файл уже снимается.
    fn try_acquire(out: &std::path::Path) -> Option<Self> {
        // Отравленный замок (паника под ним) — берём данные как есть: список
        // путей от паники не портится, а отказ снимать навсегда хуже.
        let mut running = SHOTS_RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if running.iter().any(|p| p == out) { return None; }
        running.push(out.to_path_buf());
        Some(Self(out.to_path_buf()))
    }
}

impl Drop for ShotLock {
    fn drop(&mut self) {
        let mut running = SHOTS_RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|p| p != &self.0);
    }
}

/// Браузер для снимка ссылки (порядок — в `shot_browser`). `browsers.json`
/// читается на каждый снимок, как прокси: выбор в «Браузерах» действует
/// без перезапуска.
fn pick_shot_browser(id: i64, opener: Option<&str>) -> Result<shot_browser::Choice, String> {
    let cfg     = shot_browser::parse_browsers_json(&read_config_raw("browsers.json"));
    let local   = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let pf      = std::env::var("PROGRAMFILES").unwrap_or_default();
    let pf86    = std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| pf.clone());
    let builtin = shot_browser::builtin_candidates(&local, &pf, &pf86);
    let inp = shot_browser::Inputs { opener, cfg: cfg.as_ref(), builtin: &builtin };
    shot_browser::choose(&inp, &shot_browser::system_default_browser, &shot_browser::runnable)
        .ok_or_else(|| {
            logger::log(&format!(
                "скриншот id={id}: нет браузера для снимков — папка: {}, по умолчанию: {}, Windows: {}, в списке {}, встроенных путей {}",
                opener.unwrap_or("—"),
                cfg.as_ref().map_or("нет browsers.json", |c| c.default.as_str()),
                shot_browser::system_default_browser().unwrap_or_else(|| "не определён".into()),
                cfg.as_ref().map_or(0, |c| c.list.len()),
                builtin.len()));
            t!("shot.no_browser")
        })
}

/// Один запуск браузера с одним флагом headless. `Ok` не значит «снимок
/// есть» — решает наличие файла (см. «Готовность скриншота» в CLAUDE.md).
/// `Ok(true)` — браузер не уложился в срок и снят нами: файла после этого
/// уже не будет, ждать его «запоздалой видимости» незачем.
#[allow(clippy::too_many_arguments)]
fn shoot_once(
    exe: &str, headless: &str, profile: &std::path::Path, out: &str,
    w: u32, h: u32, t: u32, proxy_arg: Option<&str>, url: &str,
) -> Result<bool, String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new(exe);
    cmd.args([
        headless,
        "--disable-gpu",
        "--no-sandbox",
        "--hide-scrollbars",
        &format!("--window-size={w},{h}"),
        &format!("--timeout={}", t * 1000),
        &format!("--user-data-dir={}", profile.to_string_lossy()),
        &format!("--screenshot={out}"),
    ]);
    // Прокси и URL — отдельно: флаг обязан идти ДО позиционного аргумента с адресом.
    if let Some(p) = proxy_arg {
        cmd.arg(format!("--proxy-server={p}"));
    }
    cmd.arg(url);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // Сразу после запуска, пока браузер не завёл детей (см. `ProcTree`).
    let tree = shot_browser::ProcTree::attach(&child);
    // Запас 8с на холодный старт браузера + teardown под нагрузкой пакета (browser --timeout = t).
    // Дедлайн на ОДНУ попытку (см. `headless_flags`).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs((t + 8) as u64);
    let mut last_size: u64 = 0;
    let mut stable = 0;
    // Снимаем всё дерево процессов, а не родителя: иначе рендереры остаются
    // висеть и держат профиль. `wait` — чтобы не оставить зомби-хэндл.
    let stop = |child: &mut std::process::Child| {
        tree.stop();
        child.wait().ok();
    };
    loop {
        // Файл готов? размер >0 и стабилен 2 замера (~500мс) → сразу забрать, не ждать выхода/дедлайна
        if let Ok(meta) = std::fs::metadata(out) {
            let sz = meta.len();
            if sz > 0 && sz == last_size {
                stable += 1;
                if stable >= 2 {
                    stop(&mut child);
                    return Ok(false);
                }
            } else {
                stable = 0;
                last_size = sz;
            }
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            // Родитель вышел сам, но дети могли остаться — снять и их. По
            // мёртвому PID `taskkill /T` их не находит, job — находит.
            Some(_) => { tree.stop(); return Ok(false); }
            None => {
                if std::time::Instant::now() >= deadline {
                    stop(&mut child);
                    std::thread::sleep(std::time::Duration::from_millis(300)); // дать дописать PNG
                    return Ok(true);                  // НЕ Err — решаем по наличию файла
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}

async fn do_screenshot(
    data_dir: std::path::PathBuf,
    id: i64,
    url: String,
    opener: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    timeout: Option<u32>,
) -> Result<Shot, String> {
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    let filename = format!("{id}.png");
    let path = data_dir.join(&filename);

    // Первым делом, до `remove_file` и запуска: второй снимок того же файла
    // отказывается сразу, не тронув первый. Держится до конца функции.
    let Some(_shot_lock) = ShotLock::try_acquire(&path) else {
        logger::log(&format!("скриншот id={id}: уже снимается, второй запуск отклонён"));
        return Err(SHOT_BUSY.to_string());
    };

    // `None` = «размер не задан явно, взять из настроек». Файл читаем ТОЛЬКО
    // когда чего-то не хватает: из `refresh_thumb` приходят все три значения,
    // и пакет на сотни ссылок не должен лишний раз трогать диск.
    let (w, h, t) = match (width, height, timeout) {
        (Some(w), Some(h), Some(t)) => (w, h, t),
        _ => {
            let (dw, dh, dt) = thumb_cfg_from_settings();
            (width.unwrap_or(dw), height.unwrap_or(dh), timeout.unwrap_or(dt))
        }
    };

    // Самая частая жалоба про скриншоты. Отказ выбора пишется в журнал внутри,
    // ДО запуска — иначе в журнале не осталось бы ничего.
    let choice = pick_shot_browser(id, opener.as_deref())?;
    let flags  = shot_browser::headless_flags(shot_browser::engine_of(&choice.exe));

    let path_str2 = path.to_string_lossy().into_owned();

    // Прокси для браузера: читаем ДО spawn_blocking — значение нужно и в
    // командной строке, и в тексте ошибки ниже, и для проверки адреса.
    let proxy_arg  = proxy_server_arg();
    let proxy_used = proxy_arg.is_some();

    // Адрес доступен? — до запуска браузера и до удаления прежнего рисунка:
    // мёртвая ссылка не стирает старую картинку и не тратит t+8 с (а у Opera 95
    // не даёт белого листа вместо ошибки). Через прокси не проверяем: трафик
    // браузера идёт туда, и прямая проверка солгала бы в обе стороны. Если
    // прокси включён, но релей не поднялся, `proxy_arg` пуст и браузер идёт
    // напрямую — тогда проверка честная.
    if !proxy_used {
        if let Some((host, port)) = shot_browser::host_port(&url) {
            let probe = tauri::async_runtime::spawn_blocking(move || {
                shot_browser::probe_host(&host, port, PROBE_BUDGET)
            }).await.map_err(|e| e.to_string())?;
            if let Err(why) = probe {
                logger::log(&format!("скриншот id={id}: адрес недоступен ({}) — браузер не запускался", why.log));
                return Err(t!("shot.unreachable", why = why.ui));
            }
        }
    }

    // Удалить старый {id}.png ДО запуска — иначе поллинг увидит прошлую картинку как «готовую».
    // Между попытками не удаляем: поздно проявившийся файл первой попытки — тоже снимок.
    let _ = std::fs::remove_file(&path);

    let t_start = std::time::Instant::now();
    let exe = choice.exe.clone();
    let url_run = url.clone();

    // Run blocking browser process on a dedicated thread so the UI stays responsive.
    // Попытки — по `headless_flags`; сейчас у всех браузеров одна. Следующая —
    // только если после предыдущей файла нет.
    // Ok((попытки, последняя снята по сроку)).
    let run: Result<(Vec<&'static str>, bool), String> = tauri::async_runtime::spawn_blocking(move || {
        let mut tried = Vec::new();
        let mut killed = false;
        for &flag in flags {
            tried.push(flag);
            // Профиль свой на каждую попытку: одна ссылка может сниматься дважды
            // разом (пакет и расширение), а убитый браузер оставляет в профиле
            // файлы блокировки, о которые споткнулась бы следующая попытка.
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let profile = std::env::temp_dir().join(format!(
                "{}{id}_{}_{stamp}", shot_browser::PROFILE_PREFIX, std::process::id()));
            let r = shoot_once(&exe, flag, &profile, &path_str2, w, h, t,
                               proxy_arg.as_deref(), &url_run);
            // Процессы уже сняты (`shoot_once`), но могут ещё держать файлы.
            // Не удалилось — останется до чистки при следующем запуске.
            if let Err(e) = shot_browser::remove_profile(&profile) {
                logger::log(&format!("скриншот id={id}: профиль {} не удалён: {e}", profile.display()));
            }
            killed = r?;
            if std::path::Path::new(&path_str2).exists() { break; }
        }
        Ok((tried, killed))
    }).await.map_err(|e| e.to_string())?;

    // Гонка видимости файла: только что записанный браузером PNG может быть не виден метаданным
    // сразу после выхода браузера (Windows Defender задерживает видимость на незнакомых путях).
    // Ждём появления файла до t секунд (настройка), прежде чем признать неудачу.
    // Только если браузер вышел сам: снятый по сроку уже ничего не допишет, а
    // лишние t секунд превращали зависание в 33 с вместо 20 (Vivaldi, 19.10.2026).
    let killed = matches!(run, Ok((_, true)));
    let mut visible = path.exists();
    if !visible && !killed {
        let max_iters = (t as u64) * 10;                   // потолок = t секунд (шаг 100мс)
        for _ in 0..max_iters {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if path.exists() { visible = true; break; }
        }
    }

    // Одна строка на снимок — и на удачный тоже (решение пользователя,
    // 19.10.2026): по журналу видно, каким браузером и за сколько снята каждая
    // ссылка и был ли браузер запасным. Строки перед запуском нет: у ожидания
    // жёсткий предел (t+8 на попытку), зависший снимок всё равно дойдёт сюда.
    let fallback_note = choice.fallback.as_ref().map(|r| format!("; запасной: {}", r.log)).unwrap_or_default();   // i18n: не переводится — журнал
    if visible {
        let flag = run.as_ref().ok().and_then(|(v, _)| v.last().copied()).unwrap_or("");
        logger::log(&format!(
            "скриншот id={id}: снят за {}мс ({}, {flag}{fallback_note})",
            t_start.elapsed().as_millis(), choice.name));
    } else {
        let tried = run.as_ref().map(|(v, _)| v.join(", ")).unwrap_or_else(|e| format!("отказ запуска: {e}"));   // i18n: не переводится — журнал
        logger::log(&format!(
            "скриншот id={id}: файла нет за {}мс ({}, {}; попытки: {tried}{}; {w}x{h}, ожидание {t}с, прокси: {}{fallback_note})",
            t_start.elapsed().as_millis(),
            choice.name, choice.exe,
            if killed { " — снят по сроку" } else { "" },
            if proxy_used { "да" } else { "нет" },
        ));
    }

    if visible {
        Ok(Shot { filename, browser: choice.name, fallback: choice.fallback.map(|m| m.ui) })
    } else {
        // Выбранный браузер снимать умеет, но файла нет — честная ошибка,
        // запасной не пробуем (решение пользователя): иначе каждая мёртвая
        // ссылка в пакете снималась бы всеми браузерами по очереди.
        let base = run.err().unwrap_or_else(|| t!("shot.failed", name = choice.name));
        if proxy_used {
            // Приписка только когда флаг реально передан: при прокси с логином
            // мы идём напрямую, и упоминание прокси было бы ложным следом.
            Err(t!("shot.failed_proxy", base))
        } else {
            Err(base)
        }
    }
}

/// Записать имя снятого файла в базу. `Ok(true)` — запись состоялась,
/// `Ok(false)` — записывать было некуда либо база сменилась, `Err` — отказ базы.
///
/// Ноль затронутых строк означает, что закладку стёрли НАВСЕГДА, пока шла
/// съёмка: снимок из расширения делается fire-and-forget до ~20 с, времени
/// хватает. Такой файл убираем здесь же — чистка снимков идёт от строк к файлам
/// (`purge_node` и `empty_trash` читают колонку `thumb`), поэтому сироту без
/// строки не найдёт уже никакая ветка, она останется навсегда.
///
/// Мягкое удаление (в корзину) сюда не попадает: строка на месте, `UPDATE` её
/// находит, файл сохраняется — восстановление вернёт закладку с картинкой.
///
/// `execute` отдаёт число строк, а не признак успеха: ни `.ok()`, ни `?` ноль
/// не отличают — потому потеря и была незаметной.
///
/// ЗАЧЕМ СВЕРКА БАЗЫ. `db_path` и `data_dir` захвачены ДО съёмки, а за её время
/// человек успевает открыть другую базу: `switch_db` подменяет и соединение, и
/// путь в этом же `AppState`, а окно не модально. Тогда `UPDATE` ушёл бы в новую
/// базу с id из старой — при попадании это запись чужой закладке, при промахе
/// удаление файла из папки прежней базы, где закладка жива. Имя снимка
/// детерминированное, так что удалили бы единственную картинку, на которую там
/// стоит ссылка. Поэтому при смене базы не делаем НИЧЕГО.
///
/// Отвергнутый вариант, чтобы его не предложили снова как более простой:
/// «перечитать `data_dir` перед удалением». Он исправляет папку, но не id — и
/// удалит `{id}.png` уже в НОВОЙ базе, где этот id принадлежит другой закладке.
/// Хуже, чем ничего.
///
/// Сверяется путь к файлу базы, а не папка данных: две базы в одном каталоге
/// делят одну `Data`, и по папке подмена была бы незаметна (см. задачу про
/// автономные контейнеры `ИмяБазы_Data`).
fn store_thumb(
    state: &AppState,
    db_path: &std::path::Path,
    data_dir: &std::path::Path,
    id: i64,
    filename: &str,
) -> Result<bool, String> {
    // ПОРЯДОК ЗАХВАТА НЕСУЩИЙ: сначала `db`, затем, НЕ отпуская его, `db_path`.
    // `switch_db` берёт их в том же порядке, поэтому пока мы держим `db`, базу
    // подменить нельзя и окна между проверкой и записью не остаётся. Обратный
    // порядок даёт классический deadlock: он держит `db` и ждёт `db_path`, мы
    // держим `db_path` и ждём `db`.
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let same_db = state.db_path.lock()
        .map(|p| p.as_path() == db_path)
        .unwrap_or(false);
    if !same_db {
        logger::log(&format!(
            "снимок id={id}: база сменилась за время съёмки, файл {filename} оставлен как есть"));
        return Ok(false);
    }

    match db::set_thumb(&conn, id, filename) {
        Ok(0) => {
            match std::fs::remove_file(data_dir.join(filename)) {
                Ok(())   => logger::log(&format!(
                    "снимок id={id}: закладка стёрта во время съёмки, файл {filename} удалён")),
                Err(err) => logger::log(&format!(
                    "снимок id={id}: закладка стёрта во время съёмки, файл {filename} удалить не удалось: {err}")),
            }
            Ok(false)
        }
        Ok(_)    => Ok(true),
        Err(err) => {
            logger::log(&format!("снимок id={id}: имя файла не записано в базу: {err}"));
            Err(err.to_string())
        }
    }
}

#[tauri::command]
async fn refresh_thumb(
    state: tauri::State<'_, AppState>,
    id: i64,
    url: String,
    width: Option<u32>,
    height: Option<u32>,
    timeout: Option<u32>,
) -> Result<Shot, String> {
    let url = normalize_url(&url);
    // Путь к базе запоминаем вместе с папкой данных: по нему `store_thumb`
    // сверит, что за время съёмки не открыли другую базу.
    let (db_path, data_dir) = {
        let p = state.db_path.lock().map_err(|e| e.to_string())?;
        (p.clone(), p.parent().ok_or("no parent dir")?.to_path_buf().join("Data"))
    };
    let (old_thumb, opener): (Option<String>, Option<String>) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        (conn.query_row("SELECT thumb FROM nodes WHERE id=?1", rusqlite::params![id], |r| r.get(0)).ok().flatten(),
         db::root_opener(&conn, id).ok().flatten())
    };
    let shot = do_screenshot(data_dir.clone(), id, url, opener, width, height, timeout).await?;
    if let Some(old) = old_thumb {
        if old != shot.filename {
            let _ = std::fs::remove_file(data_dir.join(&old));
        }
    }
    // Возврат намеренно остаётся Ok, даже если строки уже нет: съёмка удалась, а
    // закладку стёр сам человек. Иначе такие случаи попали бы в сводку пакета как
    // неудачи, а её смотрят, чтобы отличить сетевые отказы от нормальной работы.
    // Отказ самой базы по-прежнему уходит наверх.
    store_thumb(&state, &db_path, &data_dir, id, &shot.filename)?;
    Ok(shot)
}

#[tauri::command]
fn delete_node(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE nodes SET deleted=1, deleted_parent=parent, parent=NULL WHERE id=?1",
        rusqlite::params![id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn update_bookmark(
    state: tauri::State<AppState>,
    id: i64, title: String, url: String, note: String,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let note_val: Option<String> = if note.trim().is_empty() { None } else { Some(note) };
    conn.execute(
        "UPDATE nodes SET title = ?1, url = ?2, note = ?3 WHERE id = ?4",
        rusqlite::params![title, url, note_val, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct UrlCheckResult {
    pub url:      String,
    pub status:   u16,
    pub ok:       bool,
    pub timed_out: bool,
    pub redirect: Option<String>,
    pub ms:       u64,
    pub err:      Option<String>,
    pub skipped:  bool,
}

// Служебные/не-HTTP схемы (edge://, chrome://, about:, file://, mailto:, …) — не проверяются HTTP-запросом
fn is_non_http_scheme(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    const KNOWN: &[&str] = &[
        "chrome:", "edge:", "about:", "file:", "mailto:", "tel:", "data:",
        "javascript:", "ftp:", "ftps:", "view-source:", "chrome-extension:", "moz-extension:",
    ];
    if KNOWN.iter().any(|k| u.starts_with(k)) { return true; }
    if let Some(pos) = u.find("://") {
        let scheme = &u[..pos];
        let is_scheme = !scheme.is_empty()
            && scheme.chars().next().map_or(false, |c| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
        if is_scheme && scheme != "http" && scheme != "https" { return true; }
    }
    false
}

#[tauri::command]
async fn check_url(url: String) -> UrlCheckResult {
    if is_non_http_scheme(&url) {
        return UrlCheckResult { url, status: 0, ok: false, timed_out: false,
            redirect: None, ms: 0, err: None, skipped: true };
    }
    let url = normalize_url(&url);
    let t0 = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(8);
    let client = match http_client(timeout, "Mozilla/5.0 URL-Album-Checker/1.0") {
        Ok(c) => c,
        Err(e) => {
            logger::log(&format!("проверка {url}: клиент не создан: {e}"));
            return UrlCheckResult { url, status: 0, ok: false, timed_out: false,
                redirect: None, ms: 0, err: Some(e), skipped: false };
        }
    };
    let resp = match client.head(&url).send().await {
        Ok(r) if r.status().as_u16() == 405 => client.get(&url).send().await,
        other => other,
    };
    let ms = t0.elapsed().as_millis() as u64;
    match resp {
        Ok(r) => {
            let status    = r.status().as_u16();
            let final_url = r.url().to_string();
            // Ответ получен, но неудачный. Раньше такие ссылки в журнал не
            // попадали вовсе: ниже, в Err-ветке, пишутся только обрывы связи
            // и таймауты, а 404 и 500 проходили молча — в пакете человек видел
            // счётчик ошибок, но не список, какие именно не открылись.
            // Успехи по-прежнему не пишем.
            if status >= 400 {
                logger::log(&format!("проверка {url}: ответ {status}"));
            }
            UrlCheckResult {
                ok: status < 400, timed_out: false,
                redirect: (final_url != url).then_some(final_url),
                err: None, url, status, ms, skipped: false,
            }
        }
        Err(e) => {
            let timed_out = e.is_timeout();
            // Только ошибки: успешные ответы не пишем, иначе проверка папки
            // на сотню ссылок раздует журнал до нечитаемости.
            logger::log(&format!("проверка {url}: {}",
                if timed_out { "таймаут 8с".to_string() } else { short_err(&e) }));
            UrlCheckResult { url, status: 0, ok: false, timed_out,
                redirect: None, ms, err: Some(e.to_string()), skipped: false }
        }
    }
}

#[tauri::command]
fn sort_all_bookmarks(state: tauri::State<AppState>, by: String, desc: bool) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let order = db::sort_order_sql(&by, desc);
    let folder_ids: Vec<i64> = {
        let mut s = conn.prepare("SELECT id FROM nodes WHERE kind='folder'")
            .map_err(|e| e.to_string())?;
        let x = s.query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<i64>>>()
            .map_err(|e| e.to_string())?;
        x
    };
    for fid in folder_ids {
        let sql = format!(
            "SELECT id FROM nodes WHERE parent={fid}
             ORDER BY CASE kind WHEN 'folder' THEN 0 ELSE 1 END, {order}"
        );
        let ids: Vec<i64> = {
            let mut s = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let x = s.query_map([], |r| r.get::<_, i64>(0))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<i64>>>()
                .map_err(|e| e.to_string())?;
            x
        };
        for (i, id) in ids.iter().enumerate() {
            conn.execute("UPDATE nodes SET sort_idx=?1 WHERE id=?2",
                rusqlite::params![i as i64, id]).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ── Database management ───────────────────────────────────────────────────────

#[tauri::command]
fn clear_screenshots(state: tauri::State<AppState>) -> Result<usize, String> {
    let db_dir = state.db_path.lock().map_err(|e| e.to_string())?
        .parent().ok_or("no parent")?.to_path_buf();
    let data_dir = db_dir.join("Data");
    if !data_dir.exists() { return Ok(0); }
    let mut deleted = 0usize;
    for entry in std::fs::read_dir(&data_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().map(|x| x.eq_ignore_ascii_case("png")).unwrap_or(false) {
            if std::fs::remove_file(entry.path()).is_ok() { deleted += 1; }
        }
    }
    Ok(deleted)
}

/// Тело очистки базы, одной транзакцией; служебная «Новые ссылки» заводится
/// заново в той же транзакции — очищенная база не остаётся без папок.
fn clear_nodes(conn: &Connection) -> Result<(), String> {
    // Раньше здесь были ещё DELETE из entries и folders — таблиц старой схемы,
    // которых db::init не создаёт. execute_batch падал на них уже ПОСЛЕ удаления
    // nodes: данные стёрты, команда вернула ошибку, VACUUM не выполнился, и
    // пользователь решал, что очистка не сработала.
    conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    if let Err(e) = conn.execute_batch(
        "DELETE FROM nodes;
         DELETE FROM sqlite_sequence WHERE name = 'nodes';"
    ).map_err(|e| e.to_string()).and_then(|_| find_or_create_inbox_folder(conn)) {
        conn.execute_batch("ROLLBACK").ok();
        logger::log(&format!("очистка базы отменена, данные на месте: {e}"));
        return Err(e);
    }
    conn.execute_batch("COMMIT").map_err(|e| {
        logger::log(&format!("очистка базы: COMMIT не прошёл: {e}"));
        e.to_string()
    })
}

#[tauri::command]
fn clear_db(state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    clear_nodes(&conn)?;

    // VACUUM сжимает файл, и только ПОСЛЕ COMMIT — внутри транзакции он не работает.
    if let Err(e) = conn.execute_batch("VACUUM") {
        // Данные уже очищены и зафиксированы: это не отказ операции, а лишь
        // несжатый файл. Ошибку наверх не отдаём — иначе снова получим
        // «очистка не сработала» при том, что она сработала.
        logger::log(&format!("очистка базы: VACUUM не выполнен, данные очищены: {e}"));
    }
    // VACUUM может сбросить journal_mode — возвращаем WAL отдельным батчем,
    // чтобы это произошло даже при неудачном VACUUM.
    if let Err(e) = conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;") {
        logger::log(&format!("очистка базы: режим WAL не восстановлен: {e}"));
    }
    Ok(())
}

#[tauri::command]
/// Switch the active connection to a file chosen by the user.
/// Opens in-place — no copying. The chosen file becomes the active DB.
/// `false` — окно закрыли без выбора: это не ошибка, JS просто ничего не делает.
async fn open_db(state: tauri::State<'_, AppState>, _window: tauri::Window) -> Result<bool, String> {
    let start_dir = state.db_path.lock()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title(t!("rfd.open_db"))
        .add_filter(t!("rfd.filter.app_db"), &["db"])
        .set_directory(&start_dir)
        .pick_file().await
    else { return Ok(false) };
    let src = file.path().to_path_buf();

    // Guard against opening the already-active database
    let current = state.db_path.lock().map_err(|e| e.to_string())?.clone();
    if std::fs::canonicalize(&src).ok() == std::fs::canonicalize(&current).ok() {
        return Err(t!("db.already_open"));
    }

    switch_db(state, src)?;
    Ok(true)
}

/// Схема новой базы и служебная «Новые ссылки» в ней: окну «Новая ссылка»
/// нужна хотя бы одна папка.
fn init_new_db(conn: &Connection) -> Result<(), String> {
    db::init(conn).map_err(|e| e.to_string())?;
    find_or_create_inbox_folder(conn).map(|_| ())
}

/// База при старте программы. Файла не было (первый запуск) — новая база,
/// с «Новые ссылки»; в существующую, даже пустую, ничего не добавляется.
/// Ошибка — готовая строка для журнала.
fn open_startup_db(path: &std::path::Path) -> Result<Connection, String> {
    let is_new = !path.exists();
    let conn = Connection::open(path)
        .map_err(|e| format!("не удалось открыть базу {}: {e}", path.display()))?;   // i18n: не переводится — журнал (окна ещё нет)
    if is_new {
        init_new_db(&conn)
    } else {
        db::init(&conn).map_err(|e| e.to_string())
    }
    .map_err(|e| format!("не удалось подготовить базу {}: {e}", path.display()))?;   // i18n: не переводится — журнал (окна ещё нет)
    Ok(conn)
}

/// Create a new empty database at a user-chosen path, then open it.
/// `false` — окно закрыли без выбора.
#[tauri::command]
async fn create_new_db(state: tauri::State<'_, AppState>, _window: tauri::Window) -> Result<bool, String> {
    // Default directory: folder of the current active DB, or exe folder as fallback
    let start_dir = state.db_path.lock()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title(t!("rfd.new_db"))
        .add_filter(t!("rfd.filter.app_db"), &["db"])
        .set_file_name("album.db")
        .set_directory(&start_dir)
        .save_file().await
    else { return Ok(false) };
    let path = file.path().to_path_buf();

    // Create and initialise the new empty file
    let new_conn = Connection::open(&path).map_err(|e| e.to_string())?;
    init_new_db(&new_conn)?;
    // Close it so switch_db can reopen cleanly
    drop(new_conn);

    switch_db(state, path)?;
    Ok(true)
}

// ── Last-used DB persistence (portable: last_db.txt next to exe) ─────────────

fn last_db_file() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("last_db.txt"))
}

fn save_last_db(path: &std::path::Path) {
    if let Some(f) = last_db_file() {
        if let Err(e) = std::fs::write(f, path.to_string_lossy().as_bytes()) {
            logger::log(&format!("не удалось записать last_db.txt: {e}"));
        }
    }
}

fn load_last_db() -> Option<std::path::PathBuf> {
    let content = std::fs::read_to_string(last_db_file()?).ok()?;
    let p = std::path::PathBuf::from(content.trim());
    if p.exists() { Some(p) } else { None }
}

fn recent_dbs_file() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("recent_dbs.txt"))
}

fn save_recent_db(path: &std::path::Path) {
    let Some(f) = recent_dbs_file() else { return };
    let path_str = path.to_string_lossy().into_owned();
    let existing = std::fs::read_to_string(&f).unwrap_or_default();
    let entries: Vec<String> = std::iter::once(path_str.clone())
        .chain(existing.lines().filter(|l| !l.trim().is_empty() && *l != path_str).map(String::from))
        .take(10)
        .collect();
    if let Err(e) = std::fs::write(f, entries.join("\n")) {
        logger::log(&format!("не удалось записать recent_dbs.txt: {e}"));
    }
}

/// Checkpoint current connection, open a new one at `new_path`, update AppState.
/// Также зарегистрирована как IPC-команда — вызывается из «Последние базы».
#[tauri::command]
fn switch_db(state: tauri::State<'_, AppState>, new_path: std::path::PathBuf) -> Result<(), String> {
    let mut db_guard = state.db.lock().map_err(|e| e.to_string())?;
    // Checkpoint WAL of current DB before switching
    db_guard.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").ok();

    let new_conn = Connection::open(&new_path).map_err(|e| e.to_string())?;
    db::init(&new_conn).map_err(|e| e.to_string())?;
    migrate_thumb_to_filename(&new_conn, &new_path.parent().unwrap_or(&new_path).join("Data"));
    *db_guard = new_conn;

    // Путь меняется, пока `db` ещё захвачен: store_thumb и update_note сверяют
    // путь под тем же `db`, и между подменой соединения и пути у них не должно
    // быть окна, где новая база видна со старым путём.
    let mut path_guard = state.db_path.lock().map_err(|e| e.to_string())?;
    *path_guard = new_path.clone();
    drop(path_guard);
    drop(db_guard);
    save_last_db(&new_path);
    save_recent_db(&new_path);
    logger::log(&format!("открыта база: {}", new_path.display()));
    Ok(())
}

/// WAL-checkpoint перед копированием файла базы. Копируется только `*.db`,
/// сайдкар `*.db-wal` — нет, поэтому всё, что ещё лежит в WAL, в резервную
/// копию не попадёт. Возвращает текст ошибки, если checkpoint не прошёл:
/// копию в этом случае всё равно делаем — файл валиден, просто без самых
/// свежих записей.
///
/// Функция синхронная: guard живёт до конца её тела и отпускается до возврата,
/// поэтому MutexGuard заведомо не удерживается через `.await` у вызывающего.
fn checkpoint_before_copy(state: &tauri::State<'_, AppState>) -> Option<String> {
    let conn = match state.db.lock() {
        Ok(c)  => c,
        Err(e) => return Some(t!("backup.db_busy", e)),
    };
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .err()
        .map(|e| e.to_string())
}

/// Собрать возвращаемое значение резервного копирования: путь + опциональное
/// предупреждение о непройденном checkpoint. Строка целиком уходит в showNotice.
fn backup_result(path: &std::path::Path, wal_warn: Option<String>) -> String {
    let mut s = path.to_string_lossy().to_string();
    if let Some(w) = wal_warn {
        s.push_str(&t!("backup.wal_warning", w));
    }
    s
}

/// `None` — окно закрыли без выбора.
#[tauri::command]
async fn backup_db(state: tauri::State<'_, AppState>, window: tauri::Window) -> Result<Option<String>, String> {
    let (src, src_dir, src_name) = {
        let p = state.db_path.lock().map_err(|e| e.to_string())?.clone();
        let dir = p.parent().ok_or("no parent")?.to_path_buf();
        let name = p.file_name().ok_or("no filename")?.to_string_lossy().into_owned();
        (p, dir, name)
    };
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.backup"))
        .add_filter(t!("rfd.filter.db"), &["db"])
        .set_file_name(&src_name)
        .set_directory(&src_dir)
        .save_file().await
    else { return Ok(None) };

    // Диалог уже закрыт — дальше ни одного .await, guard внутри хелпера безопасен.
    let wal_warn = checkpoint_before_copy(&state);
    std::fs::copy(&src, file.path()).map_err(|e| e.to_string())?;
    Ok(Some(backup_result(file.path(), wal_warn)))
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ft = entry.file_type().map_err(|e| e.to_string())?;
        if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// `None` — окно закрыли без выбора.
#[tauri::command]
async fn backup_db_with_data(state: tauri::State<'_, AppState>, window: tauri::Window) -> Result<Option<String>, String> {
    let (db_src, db_dir, db_name) = {
        let p = state.db_path.lock().map_err(|e| e.to_string())?.clone();
        let dir = p.parent().ok_or("no parent")?.to_path_buf();
        let name = p.file_name().ok_or("no filename")?.to_string_lossy().into_owned();
        (p, dir, name)
    };
    let Some(dest_folder) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.backup_dir"))
        .set_directory(&db_dir)
        .pick_folder().await
    else { return Ok(None) };
    let dest = dest_folder.path().to_path_buf();

    // Bug 2 fix: prevent dest == db_dir which would overwrite the source DB
    let canon_dest = std::fs::canonicalize(&dest).unwrap_or_else(|_| dest.clone());
    let canon_src  = std::fs::canonicalize(&db_dir).unwrap_or_else(|_| db_dir.clone());
    if canon_dest == canon_src {
        return Err(t!("backup.same_dir"));
    }

    // Copy the DB file itself
    // Диалог уже закрыт — дальше ни одного .await, guard внутри хелпера безопасен.
    let wal_warn = checkpoint_before_copy(&state);
    std::fs::copy(&db_src, dest.join(&db_name)).map_err(|e| e.to_string())?;

    // Bug 1 fix: recursively copy Data/ so favicons/ subdirectory is included
    let data_src = db_dir.join("Data");
    if data_src.exists() {
        copy_dir_recursive(&data_src, &dest.join("Data"))?;
    }
    Ok(Some(backup_result(&dest.join(&db_name), wal_warn)))
}

#[tauri::command]
fn sort_folder(
    state: tauri::State<AppState>,
    folder_id: i64,
    by: String,
    desc: bool,
) -> Result<Vec<i64>, String> {
    let conn  = state.db.lock().map_err(|e| e.to_string())?;
    let order = db::sort_order_sql(&by, desc);
    // Folders first, then bookmarks, each group sorted by chosen column
    let sql = format!(
        "SELECT id FROM nodes WHERE parent = ?1
         ORDER BY CASE kind WHEN 'folder' THEN 0 ELSE 1 END, {order}"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let ids: Vec<i64> = stmt
        .query_map([folder_id], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<i64>>>()
        .map_err(|e| e.to_string())?;
    for (idx, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE nodes SET sort_idx = ?1 WHERE id = ?2",
            rusqlite::params![idx as i64, id],
        ).map_err(|e| e.to_string())?;
    }
    Ok(ids)
}

/// `false` — окно закрыли без выбора.
#[tauri::command]
async fn export_folder_html(state: tauri::State<'_, AppState>, window: tauri::Window, folder_id: i64) -> Result<bool, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.export_html"))
        .add_filter(t!("rfd.filter.html"), &["html", "htm"])
        .save_file().await
    else { return Ok(false) };
    let content = { let c = state.db.lock().map_err(|e| e.to_string())?; db::export_html(&c, folder_id).map_err(|e| e.to_string())? };
    std::fs::write(file.path(), content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// `false` — окно закрыли без выбора.
#[tauri::command]
async fn export_folder_txt(state: tauri::State<'_, AppState>, window: tauri::Window, folder_id: i64) -> Result<bool, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.export_txt"))
        .add_filter(t!("rfd.filter.txt"), &["txt"])
        .save_file().await
    else { return Ok(false) };
    let content = { let c = state.db.lock().map_err(|e| e.to_string())?; db::export_txt(&c, folder_id, &t!("txt.note_label")).map_err(|e| e.to_string())? };
    std::fs::write(file.path(), content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
async fn pick_browser_file(window: tauri::Window) -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .add_filter(t!("rfd.filter.exe"), &["exe"])
        .set_title(t!("rfd.pick_browser"))
        .pick_file().await
        .map(|f| f.path().to_string_lossy().into_owned())
}

/// Правка заметки на месте. `db_path` — база, в которой заметку начали
/// править: если с тех пор открыли другую, запись отклоняется, иначе текст
/// лёг бы в чужую базу, где этот id принадлежит другой ссылке.
#[tauri::command]
fn update_note(state: tauri::State<AppState>, id: i64, note: String, db_path: String) -> Result<(), String> {
    // Порядок захвата — как в store_thumb и switch_db: `db`, затем `db_path`.
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let same_db = state.db_path.lock()
        .map(|p| p.as_path() == std::path::Path::new(&db_path))
        .unwrap_or(false);
    if !same_db {
        logger::log(&format!("заметка id={id}: база сменилась во время правки, не сохранена"));
        return Err(t!("note.other_db"));
    }
    match db::update_note(&conn, id, &note) {
        Ok(0) => {
            logger::log(&format!("заметка id={id}: ссылки больше нет, не сохранена"));
            Err(t!("note.link_deleted"))
        }
        Ok(_) => Ok(()),
        Err(e) => {
            logger::log(&format!("заметка id={id}: не сохранена: {e}"));
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn search_bookmarks(
    state: tauri::State<AppState>,
    query: String,
    by_title: Option<bool>,
    by_url:   Option<bool>,
    by_note:  Option<bool>,
) -> Result<Vec<db::SearchResult>, String> {
    if query.trim().is_empty() { return Ok(vec![]); }
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::search_bookmarks(
        &conn, &query,
        by_title.unwrap_or(true),
        by_url  .unwrap_or(true),
        by_note .unwrap_or(true),
    ).map_err(|e| e.to_string())
}

#[tauri::command]
fn db_stats(state: tauri::State<AppState>) -> Result<String, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let total: i64    = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap_or(0);
    let folders: i64  = conn.query_row("SELECT COUNT(*) FROM nodes WHERE kind='folder'", [], |r| r.get(0)).unwrap_or(0);
    let books: i64    = conn.query_row("SELECT COUNT(*) FROM nodes WHERE kind='bookmark'", [], |r| r.get(0)).unwrap_or(0);
    let orphans: i64  = conn.query_row("SELECT COUNT(*) FROM nodes WHERE kind='bookmark' AND parent IS NULL", [], |r| r.get(0)).unwrap_or(0);
    Ok(format!("total={total} folders={folders} bookmarks={books} orphan_bookmarks={orphans}"))
}

// ── Favicon helpers ──────────────────────────────────────────────────────────

fn extract_domain(url: &str) -> Option<String> {
    let url = url.trim();
    let after_scheme = if let Some(pos) = url.find("://") {
        &url[pos + 3..]
    } else {
        url
    };
    let host = after_scheme.split(|c: char| c == '/' || c == '?' || c == '#').next()?;
    let host = host.split('@').last().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).trim().to_lowercase();
    if host.is_empty() { return None; }
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

fn sanitize_domain(domain: &str) -> String {
    domain.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect::<String>()
        .to_lowercase()
}

fn ext_from_content_type(ct: &str) -> &'static str {
    if ct.contains("svg")       { "svg"  }
    else if ct.contains("png")  { "png"  }
    else if ct.contains("gif")  { "gif"  }
    else if ct.contains("webp") { "webp" }
    else                        { "ico"  }
}

/// Returns true if the bytes look like a renderable image (not an HTML error page).
fn is_valid_image(bytes: &[u8]) -> bool {
    if bytes.len() < 4 { return false; }
    // PNG: \x89PNG
    if bytes.starts_with(b"\x89PNG") { return true; }
    // GIF: GIF87a / GIF89a
    if bytes.starts_with(b"GIF8") { return true; }
    // ICO: \x00\x00\x01\x00
    if bytes.starts_with(b"\x00\x00\x01\x00") { return true; }
    // ICO (cursor): \x00\x00\x02\x00
    if bytes.starts_with(b"\x00\x00\x02\x00") { return true; }
    // JPEG: \xFF\xD8
    if bytes.starts_with(b"\xFF\xD8") { return true; }
    // WebP: RIFF....WEBP
    if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" { return true; }
    // SVG: must start with <svg or <?xml (NOT <!DOCTYPE or <html — those are HTML error pages)
    let start = std::str::from_utf8(&bytes[..bytes.len().min(64)]).unwrap_or("").trim_start();
    if start.starts_with("<svg") || start.starts_with("<?xml") { return true; }
    false
}

fn find_icon_href(html: &str, base: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut pos = 0;
    while pos < lower.len() {
        let Some(offset) = lower[pos..].find("<link") else { break };
        let link_start = pos + offset;
        let end_offset = lower[link_start..].find('>').unwrap_or(0);
        let link_end   = link_start + end_offset + 1;
        let tag_lower  = &lower[link_start..link_end];
        let tag_orig   = &html[link_start..link_end.min(html.len())];

        if (tag_lower.contains("rel=\"icon\"")
            || tag_lower.contains("rel='icon'")
            || tag_lower.contains("shortcut icon")
            || tag_lower.contains("apple-touch-icon"))
            && tag_lower.contains("href=")
        {
            if let Some(href) = attr_value(tag_orig, "href") {
                if !href.is_empty() {
                    return Some(resolve_href(&href, base));
                }
            }
        }
        pos = link_end;
    }
    None
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let ltag  = tag.to_lowercase();
    let lattr = attr.to_lowercase();
    let dq = format!("{}=\"", lattr);
    if let Some(s) = ltag.find(&dq) {
        let vs = s + dq.len();
        if let Some(e) = ltag[vs..].find('"') {
            return Some(tag[vs..vs + e].to_string());
        }
    }
    let sq = format!("{}='", lattr);
    if let Some(s) = ltag.find(&sq) {
        let vs = s + sq.len();
        if let Some(e) = ltag[vs..].find('\'') {
            return Some(tag[vs..vs + e].to_string());
        }
    }
    None
}

fn resolve_href(href: &str, base: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with("//") {
        format!("https:{}", href)
    } else if href.starts_with('/') {
        format!("{}{}", base, href)
    } else {
        format!("{}/{}", base.trim_end_matches('/'), href)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Add https:// if the URL has no scheme. Never modifies the DB value.
fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() { return url.to_string(); }
    // Already has a recognised scheme — leave as-is
    let low = url.to_ascii_lowercase();
    if low.starts_with("http://")
        || low.starts_with("https://")
        || low.starts_with("ftp://")
        || low.starts_with("ftps://")
        || low.starts_with("file://")
        || low.starts_with("mailto:")
        || low.starts_with("tel:")
        || low.starts_with("data:")
    {
        return url.to_string();
    }
    format!("https://{}", url)
}

// `ShellExecuteW` из shell32 — тот же вызов, которым оболочка открывает ссылки.
//
// Возвращает HINSTANCE: больше 32 — что-то запущено, иначе само число и есть
// причина отказа. Ради этого он здесь и появился вместо `rundll32
// url.dll,FileProtocolHandler`: тот запускается всегда и завершается нулём
// независимо от того, нашёлся ли браузер, — «кликнул, и ничего не произошло»
// не оставляло следов нигде.
//
// ГРАНИЦА, проверена вызовом: незнакомая СХЕМА отказом НЕ считается. Windows
// 10/11 перехватывает её своим окном «Каким приложением открыть?» и возвращает
// успех — `zzznotascheme://x` дал код 42. Отказы приходят на файловый класс
// причин: несуществующий путь дал 2. То есть «в системе не назначен браузер»
// этим способом не диагностируется — человек увидит окно выбора от Windows.
// Обещать обратное в интерфейсе нельзя.
//
// Комментарий обычный, не `///`: doc-комментарий к `extern`-блоку не крепится,
// rustc отвечает `unused doc comment`.
#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd:          *mut std::ffi::c_void,
        lp_operation:  *const u16,
        lp_file:       *const u16,
        lp_parameters: *const u16,
        lp_directory:  *const u16,
        n_show_cmd:    i32,
    ) -> isize;
}

/// Строка для WinAPI: UTF-16 с завершающим нулём.
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Открыть цель обработчиком по умолчанию. `Ok` = обработчик найден и запущен.
///
/// Разбора аргументов оболочкой здесь нет, поэтому `&`, `?` и `=` в адресе
/// доходят целыми — ровно то, ради чего когда-то взяли rundll32 вместо
/// `cmd /c start`. В этом отношении ничего не потеряно.
#[cfg(windows)]
fn shell_open(target: &str) -> Result<(), i18n::Msg> {
    const SW_SHOWNORMAL: i32 = 1;
    let op   = wide("open");
    let file = wide(target);
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if code > 32 {
        return Ok(());
    }
    Err(match code {
        2  => t_msg!("open.reason.no_handler"),
        3  => t_msg!("open.reason.no_path"),
        5  => t_msg!("open.reason.access_denied"),
        8  => t_msg!("open.reason.no_memory"),
        26 => t_msg!("open.reason.busy"),
        27 => t_msg!("open.reason.assoc_incomplete"),
        31 => t_msg!("open.reason.no_assoc"),
        _  => t_msg!("open.reason.code", code),
    })
}

/// Открыть ссылку браузером по умолчанию.
///
/// `async` + `spawn_blocking`: `ShellExecuteW` возвращает управление после того,
/// как оболочка разобрала цель и запустила обработчик. Обычно это миллисекунды,
/// но на недоступном сетевом пути может встать надолго, а держать IPC-поток
/// нельзя — интерфейс замёрзнет.
#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    let url = normalize_url(&url);
    #[cfg(windows)]
    {
        let target = url.clone();
        let res = tauri::async_runtime::spawn_blocking(move || shell_open(&target))
            .await
            .map_err(|e| e.to_string())?;
        if let Err(reason) = res {
            // Отказы пишем всегда: без этого «кликнул, ничего не произошло»
            // не оставляет следов вовсе. Успехи не пишем — иначе журнал
            // раздувается на каждой ссылке (то же правило, что у check_url).
            logger::log(&format!("не удалось открыть ссылку {url}: {}", reason.log));
            return Err(t!("open.url_failed", reason = reason.ui));
        }
    }
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(&url).spawn().map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(&url).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Открыть локальный файл программой по умолчанию.
///
/// Тот же `ShellExecuteW`, что и для ссылок, и по той же причине: `cmd /c start`
/// удаётся ВСЕГДА — сам `spawn` успешен, а отказ происходит уже внутри `cmd`,
/// куда нам не видно. Здесь отсутствующий файл виден сразу: проверено, код 2.
#[tauri::command]
async fn open_file(path: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let target = path.clone();
        let res = tauri::async_runtime::spawn_blocking(move || shell_open(&target))
            .await
            .map_err(|e| e.to_string())?;
        if let Err(reason) = res {
            logger::log(&format!("не удалось открыть файл {path}: {}", reason.log));
            return Err(t!("open.file_failed", path, reason = reason.ui));
        }
    }
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(&path).spawn().map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn open_url_with(url: String, browser: String) -> Result<(), String> {
    let url = normalize_url(&url);
    if browser == "default" {
        return open_url(url).await;
    }

    // Путь проверяем ДО запуска. Портативный браузер живёт на флешке или
    // в папке, которую могли унести, переименовать или удалить; `spawn` в этом
    // случае вернёт системный текст вроде «не удаётся найти указанный файл»,
    // где не сказано ни какой файл, ни при чём тут вообще браузер.
    if !std::path::Path::new(&browser).exists() {
        logger::log(&format!("браузер не найден: {browser}"));
        return Err(t!("open.browser_missing", browser));
    }

    std::process::Command::new(&browser)
        .arg(&url)
        .spawn()
        .map_err(|e| {
            logger::log(&format!("не удалось запустить {browser}: {e}"));
            t!("open.browser_failed", browser, e)
        })?;
    Ok(())
}

// ── Generic file save ────────────────────────────────────────────────────────

/// `false` — окно закрыли без выбора.
#[tauri::command]
async fn save_text_file(window: tauri::Window, content: String, default_name: Option<String>) -> Result<bool, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.save_file"))
        .add_filter(t!("rfd.filter.txt"), &["txt"])
        .set_file_name(default_name.as_deref().unwrap_or("export.txt"))
        .save_file().await
    else { return Ok(false) };
    std::fs::write(file.path(), content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

// ── Settings (portable) ──────────────────────────────────────────────────────

/// Прочитать `settings.json` как есть. Вынесено из команды `load_settings`:
/// `#[tauri::command]` не позволяет пометить саму команду `pub(crate)` —
/// генерируемые ею макросы конфликтуют по имени. Модулям (`logger`, `relay`)
/// нужен доступ к содержимому, а знание о том, где лежит файл, должно
/// оставаться в одном месте.
/// «Не допускать дублей» (`noDuplicateUrls`) — читается из settings.json на
/// каждый вызов, как прокси: включил в настройках — следующая ссылка из
/// расширения уже проверяется, без перезапуска. Файл не разобран — false,
/// как умолчание в app.js.
fn no_duplicate_urls_from_settings() -> bool {
    serde_json::from_str::<serde_json::Value>(&read_settings_raw()).ok()
        .and_then(|v| v.get("noDuplicateUrls").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

pub(crate) fn read_settings_raw() -> String {
    read_config_raw("settings.json")
}

/// Прочитать файл настроек рядом с exe, срезав BOM.
///
/// BOM ломает serde_json МОЛЧА — теряются и настройки прокси, и флаг журнала.
/// Пользователь получает BOM, просто сохранив файл блокнотом. То же лечит и
/// JS-сторону: `load_settings` отдаёт эту же строку, а `JSON.parse` на BOM тоже
/// падает — то есть слетали вообще все настройки. (Проверено, сессия 20.)
///
/// Через эту функцию обязаны читаться ВСЕ три файла настроек: `toolbar.json` и
/// `browsers.json` разбираются в JS ровно тем же `JSON.parse` и ломались от BOM
/// точно так же, просто теряли не прокси, а панель и список браузеров.
fn read_config_raw(name: &str) -> String {
    let raw = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    raw.trim_start_matches('\u{feff}').to_string()
}

#[tauri::command]
fn load_settings() -> String {
    read_settings_raw()
}

/// Языки для «Настроек»: вшитые и `lang\`, и язык пункта «Авто».
#[tauri::command]
fn list_languages() -> i18n::LangList {
    i18n::list_languages()
}

#[tauri::command]
fn save_settings(json: String) -> Result<(), String> {
    let path = std::env::current_exe().map_err(|e| e.to_string())?
        .parent().ok_or("no parent")?.join("settings.json");
    std::fs::write(path, json.as_bytes()).map_err(|e| e.to_string())
}

// ── Toolbar config (portable) ────────────────────────────────────────────────

#[tauri::command]
fn load_toolbar_config() -> String {
    read_config_raw("toolbar.json")
}

#[tauri::command]
fn save_toolbar_config(json: String) -> Result<(), String> {
    let path = std::env::current_exe().map_err(|e| e.to_string())?
        .parent().ok_or("no parent")?.join("toolbar.json");
    std::fs::write(path, json.as_bytes()).map_err(|e| e.to_string())
}

// ── Move node (drag & drop) ──────────────────────────────────────────────────

#[tauri::command]
fn move_node(state: tauri::State<AppState>, id: i64, new_parent: Option<i64>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Guard: cannot move into self (only meaningful when target is a real node)
    if let Some(np) = new_parent {
        if id == np {
            return Err("Cannot move a folder into itself".into());
        }
    }

    // Guard: circular reference — walk up from new_parent, reject if we hit id
    // Root (None) has no ancestors, so no circular ref is possible.
    if let Some(np) = new_parent {
        let mut cur_opt: Option<i64> = Some(np);
        while let Some(cur) = cur_opt {
            if cur == id { return Err("Circular reference detected".into()); }
            cur_opt = conn
                .query_row("SELECT parent FROM nodes WHERE id = ?1", [cur], |r| r.get::<_, Option<i64>>(0))
                .ok()
                .flatten();
        }
    }

    // Place at end of new parent's children (IS handles NULL correctly in SQLite)
    let max: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_idx), -1) FROM nodes WHERE parent IS ?1",
        rusqlite::params![new_parent], |r| r.get(0),
    ).unwrap_or(-1);

    conn.execute(
        "UPDATE nodes SET parent = ?1, sort_idx = ?2 WHERE id = ?3",
        rusqlite::params![new_parent, max + 1, id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

// ── Sort index ───────────────────────────────────────────────────────────────

#[tauri::command]
fn set_sort_idx(state: tauri::State<AppState>, id: i64, sort_idx: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("UPDATE nodes SET sort_idx = ?1 WHERE id = ?2", rusqlite::params![sort_idx, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Create nodes ─────────────────────────────────────────────────────────────

#[tauri::command]
fn create_folder(state: tauri::State<AppState>, parent_id: Option<i64>, title: String) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::insert_folder(&conn, parent_id, &title).map_err(|e| e.to_string())
}

/// Служебная папка — по признаку `role`, не по названию: переименованная
/// остаётся служебной, папка человека с тем же названием ею не становится.
/// Перенесённая внутрь другой папки или удалённая в корзину служебной быть
/// перестаёт: заводится новая, признак переходит к ней.
fn find_or_create_inbox_folder(conn: &Connection) -> Result<i64, String> {
    use rusqlite::OptionalExtension;
    if let Some(id) = conn.query_row(
        // deleted: папка в корзине тоже с parent = NULL — с виду корневая.
        "SELECT id FROM nodes WHERE role = ?1 AND kind = 'folder' AND parent IS NULL \
         AND COALESCE(deleted, 0) = 0",
        rusqlite::params![db::ROLE_INBOX],
        |r| r.get::<_, i64>(0),
    ).optional().map_err(|e| e.to_string())? {
        return Ok(id);
    }
    // SAVEPOINT, а не BEGIN: зовётся и внутри транзакции clear_nodes.
    // Признак снимается со старой и ставится новой вместе — или никак.
    conn.execute_batch("SAVEPOINT inbox").map_err(|e| e.to_string())?;
    let res = (|| -> rusqlite::Result<i64> {
        conn.execute("UPDATE nodes SET role = NULL WHERE role = ?1", [db::ROLE_INBOX])?;
        let id = db::insert_folder(conn, None, &t!("folder.inbox"))?;
        conn.execute("UPDATE nodes SET role = ?1 WHERE id = ?2", rusqlite::params![db::ROLE_INBOX, id])?;
        Ok(id)
    })();
    match res {
        Ok(id) => {
            conn.execute_batch("RELEASE inbox").map_err(|e| e.to_string())?;
            Ok(id)
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK TO inbox; RELEASE inbox").ok();
            Err(e.to_string())
        }
    }
}

/// Ответ `GET /api/v1/folders`: корневые живые папки в порядке дерева.
/// `inbox: true` — служебная: расширение выбирает её по признаку, а не по
/// названию (название переводится и переименовывается).
fn extension_folders(conn: &Connection) -> rusqlite::Result<serde_json::Value> {
    let mut stmt = conn.prepare(
        "SELECT id, title, role IS ?1 FROM nodes \
         WHERE kind='folder' AND parent IS NULL \
         AND (deleted IS NULL OR deleted=0) ORDER BY sort_idx"
    )?;
    let rows = stmt.query_map([db::ROLE_INBOX], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
    })?;
    Ok(serde_json::Value::Array(
        rows.filter_map(|r| r.ok())
            .map(|(id, title, inbox)| serde_json::json!({"id": id, "title": title, "inbox": inbox}))
            .collect()
    ))
}

/// Папка для окна «Новая ссылка», когда в нём не выбрано ни одной (в базе нет
/// папок): служебная «Новые ссылки», при необходимости заводится.
#[tauri::command]
fn inbox_folder(state: tauri::State<AppState>) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    find_or_create_inbox_folder(&conn)
}

#[tauri::command]
fn create_bookmark(
    state: tauri::State<AppState>,
    parent_id: i64,
    title: String,
    url: String,
    note: Option<String>,
) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let note_val: Option<String> = note.filter(|s| !s.trim().is_empty());
    db::insert_bookmark(&conn, Some(parent_id), &title, &url, note_val.as_deref())
        .map_err(|e| e.to_string())
}

// ── DB utilities ─────────────────────────────────────────────────────────────

#[tauri::command]
fn get_db_path(state: tauri::State<AppState>) -> String {
    state.db_path.lock()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[tauri::command]
fn set_window_title(window: tauri::Window, title: String) {
    window.set_title(&title).ok();
}

/// Спрятать окно в трей. Окно не закрывается — процесс живёт, иконка в трее
/// и глобальный хоткей F8 продолжают работать; возврат — «Показать URL-Album».
#[tauri::command]
fn hide_window(window: tauri::Window) {
    window.hide().ok();
}

/// Включить/выключить журнал без перезапуска — зовётся при сохранении настроек.
#[tauri::command]
fn set_log_enabled(enabled: bool) {
    logger::set_enabled(enabled);
}

/// Записать строку в журнал со стороны интерфейса. Нужна, чтобы ошибки JS
/// не терялись: `console.error` в release-сборке недоступен, DevTools нет.
#[tauri::command]
fn log_from_ui(message: String) {
    logger::log(&format!("UI: {message}"));
}

/// Настройки оказались испорчены и были сброшены при старте (см.
/// `load_or_init_token`). UI спрашивает об этом командой `settings_were_reset`:
/// сам он ничего не заметит — к моменту загрузки фронтенда файл уже валиден.
static SETTINGS_WERE_RESET: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Момент, когда окно последний раз теряло фокус.
static LAST_UNFOCUS: std::sync::Mutex<Option<std::time::Instant>> =
    std::sync::Mutex::new(None);

/// Сколько времени после потери фокуса окно всё ещё считается активным.
/// Подобрано под задержку между кликом по значку в трее и доставкой события:
/// Windows успевает отобрать фокус раньше, чем обработчик получит `Click`, и
/// `is_focused()` там уже возвращает false. Менять здесь, значение одно.
const TRAY_RECENT_UNFOCUS: std::time::Duration = std::time::Duration::from_millis(300);

/// Файлы настроек, которые UI имеет право отложить как `.bad`.
///
/// Список закрытый намеренно: имя приходит от фронтенда, и без проверки командой
/// можно было бы попросить скопировать рядом с программой любой файл, до которого
/// дотянется `..\`.
const CONFIG_FILES: [&str; 3] = ["settings.json", "toolbar.json", "browsers.json"];

/// Отложить в сторону испорченный файл настроек: копия рядом, `<имя>.json.bad`,
/// прежняя перезаписывается. Копия, а не переименование — исходный файл не
/// трогаем, вдруг он ещё пригодится.
///
/// Для `browsers.json` это единственный способ вернуть **вручную добавленные
/// пути к портативным браузерам и их подписи**: автоматическое обнаружение
/// находит только установленные в системе, а браузер с флешки человек вписывал
/// руками — восстановить такое неоткуда, только глазами из `.bad`.
fn backup_bad_config_file(name: &str) -> Result<std::path::PathBuf, String> {
    if !CONFIG_FILES.contains(&name) {
        return Err(format!("неизвестный файл настроек: {name}"));   // i18n: не переводится — журнал (JS пишет отказ в журнал)
    }
    let src = std::env::current_exe().map_err(|e| e.to_string())?
        .parent().ok_or("no parent")?.join(name);
    if !src.exists() {
        return Err(format!("{name} не найден"));   // i18n: не переводится — журнал (JS пишет отказ в журнал)
    }
    let dst = src.with_extension("json.bad");

    // Читаем БАЙТАМИ, не строкой: файл на то и испорчен, что мог оказаться
    // не-UTF-8, а копию человек должен получить в любом случае.
    let data = std::fs::read(&src).map_err(|e| e.to_string())?;
    let (data, masked) = if name == "settings.json" {
        match mask_token_bytes(&data) {
            Some(m) => (m, true),
            None    => (data, false),
        }
    } else {
        (data, false)
    };
    std::fs::write(&dst, &data).map_err(|e| e.to_string())?;

    logger::log(&format!(
        "испорченный {name} сохранён как {}{}",
        dst.display(),
        if name != "settings.json" { "" }
        else if masked { " (токен расширения замаскирован)" }
        else { " (токен расширения найти не удалось — копия как есть)" }
    ));
    Ok(dst)
}

/// Заменить значение `extensionToken` на `***`.
///
/// Файл `.bad` человек прикладывает к сообщению на форуме, а токен даёт доступ
/// к локальному HTTP-серверу добавления закладок — в переписке ему не место.
///
/// Работаем по байтам и без разбора JSON: файл сюда попадает именно потому, что
/// разобрать его не удалось. Ключ ASCII, поэтому находится и в сломанной
/// кодировке. `None` — ключа или значения на месте нет; тогда копия уходит как
/// есть, и это честнее, чем потерять её целиком.
fn mask_token_bytes(data: &[u8]) -> Option<Vec<u8>> {
    const KEY: &[u8] = b"\"extensionToken\"";
    let pos = data.windows(KEY.len()).position(|w| w == KEY)?;

    let mut i = pos + KEY.len();
    while i < data.len() && (data[i] == b' ' || data[i] == b'\t' || data[i] == b':') {
        i += 1;
    }
    if i >= data.len() || data[i] != b'"' {
        return None;
    }
    let start = i + 1;
    let end = start + data[start..].iter().position(|&b| b == b'"')?;

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..start]);
    out.extend_from_slice(b"***");
    out.extend_from_slice(&data[end..]);
    Some(out)
}

/// Обёртка для `load_or_init_token`: там имя файла известно заранее.
fn backup_bad_settings_file() -> Result<std::path::PathBuf, String> {
    backup_bad_config_file("settings.json")
}

#[tauri::command]
fn backup_bad_config(file: String) -> Result<String, String> {
    backup_bad_config_file(&file).map(|p| p.to_string_lossy().into_owned())
}

/// Были ли настройки сброшены при старте из-за нечитаемого файла.
#[tauri::command]
fn settings_were_reset() -> bool {
    SETTINGS_WERE_RESET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Путь к журналу для кнопки «Открыть журнал». `None` — файла ещё нет.
/// Отсутствие сообщаем именно здесь: `open_file` на несуществующем пути ошибки
/// НЕ вернёт (spawn `cmd` успешен в любом случае), полагаться на него нельзя.
#[tauri::command]
fn get_log_path() -> Option<String> {
    let p = logger::log_path()?;
    p.exists().then(|| p.to_string_lossy().into_owned())
}

/// Force WAL checkpoint — incorporate all WAL data into the main DB file.
#[tauri::command]
fn checkpoint_db(state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA wal_checkpoint(FULL)").map_err(|e| e.to_string())
}

/// Checkpoint WAL with TRUNCATE (JS handles UI transition to welcome screen).
#[tauri::command]
fn close_db(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").ok();
    Ok(())
}

/// Return list of recently opened DB paths (filtered to existing files, max 10).
#[tauri::command]
fn get_recent_dbs() -> Vec<String> {
    let Some(f) = recent_dbs_file() else { return Vec::new() };
    std::fs::read_to_string(f)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && std::path::Path::new(l).exists())
        .map(String::from)
        .collect()
}

#[derive(serde::Serialize)]
struct DbProperties {
    path: String,
    size_bytes: u64,
    folder_count: i64,
    bookmark_count: i64,
}

/// Return structured DB info: path, size, folder/bookmark counts.
#[tauri::command]
fn get_db_properties(state: tauri::State<'_, AppState>) -> Result<DbProperties, String> {
    let path = state.db_path.lock().map_err(|e| e.to_string())?.clone();
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let folder_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE kind='folder'", [], |r| r.get(0)
    ).unwrap_or(0);
    let bookmark_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE kind='bookmark'", [], |r| r.get(0)
    ).unwrap_or(0);
    Ok(DbProperties {
        path: path.to_string_lossy().into_owned(),
        size_bytes,
        folder_count,
        bookmark_count,
    })
}

// ── URL scheme (urlalbum://) ─────────────────────────────────────────────────

/// Register urlalbum:// protocol handler in HKCU (no admin required).
///
/// С 2.3.3 при запуске НЕ вызывается: README обещает «не пишет в реестр», а
/// протоколом никто не пользуется (расширение ходит через HTTP-сервер).
/// Разбор ссылки (`parse_url_scheme`, `pending_open`, вторая копия) оставлен —
/// ключ, записанный прежними версиями, продолжает работать. Функция — на
/// случай, если регистрацию вернут явной командой, а не молча при старте.
#[allow(dead_code)]
fn register_url_scheme() {
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => return,
    };
    let cmd_val = format!("\"{}\" \"%1\"", exe);
    let entries: &[(&str, Option<&str>, &str)] = &[
        ("HKCU\\Software\\Classes\\urlalbum",                      None,              "URL:URL Album Protocol"),
        ("HKCU\\Software\\Classes\\urlalbum",                      Some("URL Protocol"), ""),
        ("HKCU\\Software\\Classes\\urlalbum\\shell\\open\\command", None,              &cmd_val),
    ];
    for (key, vname, data) in entries {
        let mut c = std::process::Command::new("reg");
        c.args(["add", key, "/f"]);
        if let Some(v) = vname { c.args(["/v", v]); } else { c.arg("/ve"); }
        c.args(["/d", data]);
        #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
        c.output().ok();
    }
}

/// Parse urlalbum://add?url=...&title=... into (url, title).
fn parse_url_scheme(arg: &str) -> Option<(String, String)> {
    let rest = arg.strip_prefix("urlalbum://add?")?;
    let mut url = String::new();
    let mut title = String::new();
    for part in rest.split('&') {
        if let Some(v) = part.strip_prefix("url=") {
            url = urlencoding_decode(v);
        } else if let Some(v) = part.strip_prefix("title=") {
            title = urlencoding_decode(v);
        }
    }
    if url.is_empty() { return None; }
    Some((url, title))
}

/// Раскодировать процентные последовательности.
///
/// Копим БАЙТЫ, а не символы. Одна буква кириллицы — это две процентные пары
/// (`%D0%9F`), собрать из них символ можно только на уровне байтов. Прежняя
/// версия писала `out.push(b as char)` в `String`: приведение `u8 as char` в
/// Rust задано как Latin-1 — код символа равен значению байта, — после чего
/// `String` кодировал этот символ обратно в UTF-8 уже двумя байтами. Двойная
/// перекодировка, «Проверка» превращалась в «ÐŸÑ€Ð¾Ð²ÐµÑ€ÐºÐ°». ASCII при этом
/// уцелевал, поэтому адрес выглядел исправным, а название — нет.
fn urlencoding_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let bytes = s.as_bytes();

    // Своя таблица вместо `from_str_radix`: та принимает «+f» как знаковое
    // число, и разбор зависел бы от того, что `+` уже заменён выше.
    let hex = |b: u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };

    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        // Неверная пара (`%zz`) или обрыв в конце строки: отдаём сам `%` и
        // сдвигаемся на один байт — следующие два разберутся своим чередом.
        // Прежняя версия снимала их с итератора до проверки и теряла молча.
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

#[derive(serde::Serialize)]
struct PendingOpen { url: String, title: String }

#[tauri::command]
fn get_pending_url(state: tauri::State<'_, AppState>) -> Option<PendingOpen> {
    let mut p = state.pending_open.lock().ok()?;
    p.take().map(|(url, title)| PendingOpen { url, title })
}

#[tauri::command]
fn set_extension_add_mode(state: tauri::State<'_, AppState>, mode: String) {
    if let Ok(mut m) = state.extension_add_mode.lock() {
        *m = mode;
    }
}

// Чёрный список: точные строки, которые успешно регистрируются, но ломают копирование/вставку и т.п.
fn is_blacklisted(combo: &str) -> bool {
    matches!(combo.to_lowercase().as_str(),
        "ctrl+c" | "ctrl+v" | "ctrl+x" | "ctrl+a" | "ctrl+z" | "ctrl+s")
}

#[tauri::command]
fn set_hotkey(state: tauri::State<AppState>, app: tauri::AppHandle, combo: Option<String>) -> Result<(), String> {
    let gs = app.global_shortcut();

    // [ЧТЕНИЕ — единственное] текущее значение в клон, lock сразу отпускаем
    let current = state.user_hotkey.lock().map_err(|e| e.to_string())?.clone();

    // Пусто/None → снять старый, очистить (остаётся встроенный F8)
    let new = match combo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            if let Some(old) = current.as_deref() {
                if let Err(e) = gs.unregister(old) {
                    // Не снялась — комбинация продолжит срабатывать, и активными
                    // окажутся сразу две. Симптом: «старая клавиша всё ещё работает».
                    logger::log(&format!("хоткей {old} не снят: {e}"));
                }
            }
            *state.user_hotkey.lock().map_err(|e| e.to_string())? = None;
            return Ok(());
        }
        Some(s) => s.to_string(),
    };

    // Равен текущему → ничего не делаем (избегаем лишних операций/ложных ошибок при повторном applySettings)
    if current.as_deref().map_or(false, |c| c.eq_ignore_ascii_case(&new)) {
        return Ok(());
    }

    // Валидация (тексты для showNotice)
    if new.eq_ignore_ascii_case("F8") {
        return Err(t!("hotkey.f8_reserved"));
    }
    let low = new.to_lowercase();
    if !(low.contains("ctrl") || low.contains("shift") || low.contains("alt")) {
        return Err(t!("hotkey.need_mod"));
    }
    if is_blacklisted(&new) {
        return Err(t!("hotkey.system"));
    }

    // Атомарно (lock НЕ держим): СНАЧАЛА регистрируем новый — старый трогаем только при успехе
    gs.on_shortcut(new.as_str(), |app, _shortcut, event| {
        if event.state() == ShortcutState::Pressed {
            add_from_clipboard(app);
        }
    }).map_err(|e| {
        logger::log(&format!("хоткей {new} не зарегистрирован: {e}"));
        t!("hotkey.register_failed")
    })?;

    // Успех → снять старый и записать новый (берём lock заново)
    if let Some(old) = current.as_deref() {
        if let Err(e) = gs.unregister(old) {
            logger::log(&format!("хоткей {old} не снят: {e}"));
        }
    }
    *state.user_hotkey.lock().map_err(|e| e.to_string())? = Some(new);
    Ok(())
}

// ── Browser detection ────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
struct DetectedBrowser {
    id:             String,
    name:           String,
    kind:           String, // "chromium" | "firefox"
    bookmarks_path: String,
}

#[derive(serde::Serialize)]
struct ImportSummary { links: usize, folders: usize }


fn detect_browsers_list() -> Vec<DetectedBrowser> {
    let local   = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let pf      = std::env::var("PROGRAMFILES").unwrap_or_default();
    let pf86    = std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| pf.clone());
    let mut out: Vec<DetectedBrowser> = Vec::new();

    // ── Chromium-based (LOCALAPPDATA) ──
    let local_apps: &[(&str, &str, &str)] = &[
        ("chrome",   "Google Chrome",  "Google\\Chrome"),
        ("edge",     "Microsoft Edge", "Microsoft\\Edge"),
        ("brave",    "Brave",          "BraveSoftware\\Brave-Browser"),
        ("vivaldi",  "Vivaldi",        "Vivaldi"),
        ("chromium", "Chromium",       "Chromium"),
    ];
    for (id, name, rel) in local_apps {
        for base in &[&local as &str, &pf, &pf86] {
            let user_data = format!("{}\\{}\\User Data", base, rel);
            // Собрать все профили с файлом Bookmarks (Default, Profile 1/2, кастомные)
            let mut profiles: Vec<(String, String)> = Vec::new();   // (имя папки, путь к Bookmarks)
            if let Ok(entries) = std::fs::read_dir(&user_data) {
                for entry in entries.filter_map(|e| e.ok()) {
                    if !entry.path().is_dir() { continue; }
                    let bm = entry.path().join("Bookmarks");
                    if bm.exists() {                                  // нет Bookmarks → не профиль с закладками (System/Guest отсекаются)
                        profiles.push((entry.file_name().to_string_lossy().into_owned(),
                                       bm.to_string_lossy().into_owned()));
                    }
                }
            }
            if profiles.is_empty() { continue; }                     // в этом base нет — пробуем следующий
            // Default первым, затем Profile 1, 2, … по имени
            profiles.sort_by(|a, b| {
                let rank = |s: &str| if s == "Default" { 0 } else { 1 };
                rank(&a.0).cmp(&rank(&b.0)).then(a.0.cmp(&b.0))
            });
            for (folder, bm_path) in profiles {
                let pid = format!("{id}__{folder}");                 // уникальный id на профиль
                if out.iter().any(|b| b.id == pid) { continue; }
                out.push(DetectedBrowser {
                    id: pid,
                    name: format!("{name} — {folder}"),              // «Microsoft Edge — Default» / «Brave — Profile 1»
                    kind: "chromium".to_string(),
                    bookmarks_path: bm_path,
                });
            }
            break;                                                   // профили найдены — другие base не сканируем
        }
    }

    // ── Opera family — scan entire Opera Software folder (APPDATA & LOCALAPPDATA) ──
    for base in &[&roaming as &str, &local] {
        let opera_base = format!("{}\\Opera Software", base);
        if let Ok(entries) = std::fs::read_dir(&opera_base) {
            for entry in entries.filter_map(|e| e.ok()) {
                if !entry.path().is_dir() { continue; }
                // Try all three profile layout variants
                let profile_dir = entry.path();
                let candidates = vec![
                    profile_dir.join("Bookmarks"),
                    profile_dir.join("Default").join("Bookmarks"),
                    profile_dir.join("User Data").join("Default").join("Bookmarks"),
                ];
                for bm in candidates {
                    if bm.exists() {
                        let bm_str = bm.to_string_lossy().into_owned();
                        if out.iter().any(|b| b.bookmarks_path == bm_str) { break; }
                        let dir_name = entry.file_name().to_string_lossy().into_owned();
                        let id = format!("opera_{}", out.len());
                        out.push(DetectedBrowser {
                            id, name: dir_name,
                            kind: "chromium".to_string(),
                            bookmarks_path: bm_str,
                        });
                        break;
                    }
                }
            }
        }
    }

    // ── Firefox / Waterfox / LibreWolf etc. (APPDATA) ──
    let ff_bases: &[(&str, &str)] = &[
        ("Mozilla\\Firefox",  "Mozilla Firefox"),
        ("Waterfox",          "Waterfox"),
        ("LibreWolf",         "LibreWolf"),
        ("Pale Moon",         "Pale Moon"),
        ("SeaMonkey",         "SeaMonkey"),
    ];
    for (rel, name) in ff_bases {
        let base = format!("{}\\{}", roaming, rel);
        if let Some(places) = find_gecko_places(&base) {
            let id = rel.replace('\\', "_").to_lowercase();
            if !out.iter().any(|b| b.bookmarks_path == places) {
                out.push(DetectedBrowser { id, name: name.to_string(),
                    kind: "firefox".to_string(), bookmarks_path: places });
            }
        }
    }

    out
}

fn find_gecko_places(browser_base: &str) -> Option<String> {
    // Check profiles.ini for default profile
    let ini_path = format!("{}\\profiles.ini", browser_base);
    let ini = std::fs::read_to_string(&ini_path).ok()?;
    let (mut cur_path, mut cur_rel, mut cur_def) = (String::new(), false, false);
    let mut best: Option<(String, bool)> = None;

    for line in ini.lines().map(str::trim).chain(std::iter::once("[END]")) {
        if line.starts_with('[') {
            if !cur_path.is_empty() {
                let full = if cur_rel {
                    format!("{}\\{}", browser_base, cur_path.replace('/', "\\"))
                } else { cur_path.clone() };
                let places = format!("{}\\places.sqlite", full);
                if std::path::Path::new(&places).exists() {
                    match best {
                        None => { best = Some((places, cur_def)); }
                        Some((_, false)) if cur_def => { best = Some((places, true)); }
                        _ => {}
                    }
                }
            }
            cur_path.clear(); cur_rel = false; cur_def = false;
        } else if let Some(v) = line.strip_prefix("Path=") {
            cur_path = v.trim().to_string();
        } else if line == "Default=1" { cur_def = true; }
          else if line == "IsRelative=1" { cur_rel = true; }
    }
    best.map(|(p, _)| p)
}

// ── Import from arbitrary bookmarks file / profile folder ───────────────────

#[tauri::command]
async fn pick_bookmarks_file(window: tauri::Window) -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.pick_bookmarks"))
        .add_filter(t!("rfd.filter.bookmarks"), &["json", "sqlite"])
        .pick_file().await
        .map(|f| f.path().to_string_lossy().into_owned())
}

#[tauri::command]
async fn pick_profile_folder(window: tauri::Window) -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.pick_profile"))
        .pick_folder().await
        .map(|f| f.path().to_string_lossy().into_owned())
}

/// Scan a folder for a known bookmarks file. Returns (path, kind) where kind is "chromium"|"firefox".
#[tauri::command]
fn find_bookmarks_in_folder(folder: String) -> Option<serde_json::Value> {
    use serde_json::json;
    let base = std::path::Path::new(&folder);
    // Chromium layout variants
    for rel in &["Bookmarks", "Default\\Bookmarks", "User Data\\Default\\Bookmarks"] {
        let p = base.join(rel);
        if p.exists() {
            return Some(json!({ "path": p.to_string_lossy(), "kind": "chromium" }));
        }
    }
    // Firefox/Gecko layout variants
    for rel in &["places.sqlite", "default\\places.sqlite"] {
        let p = base.join(rel);
        if p.exists() {
            return Some(json!({ "path": p.to_string_lossy(), "kind": "firefox" }));
        }
    }
    None
}

#[tauri::command]
fn import_from_bookmarks_file(
    state: tauri::State<AppState>,
    path: String,
    name: String,
) -> Result<ImportSummary, String> {
    let filename = std::path::Path::new(&path)
        .file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let (links, folders) = if filename == "places.sqlite" || filename.ends_with(".sqlite") {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::import_firefox(&conn, &path, &name).map_err(|e| e.to_string())?
    } else {
        let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::import_chromium(&conn, &json, &name).map_err(|e| e.to_string())?
    };
    Ok(ImportSummary { links, folders })
}

// ── Browser EXE detection (for Open With / Browser Manager) ─────────────────

#[derive(serde::Serialize)]
struct BrowserExe { name: String, path: String }

fn exe_exists(path: &str) -> bool { std::path::Path::new(path).exists() }

fn reg_query_cmd(key: &str) -> Option<String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new("reg");
    cmd.args(["query", key, "/ve"]);
    #[cfg(windows)] cmd.creation_flags(0x0800_0000);
    let out = cmd.output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        for ty in &["REG_SZ", "REG_EXPAND_SZ"] {
            if let Some(idx) = line.find(ty) {
                let val = line[idx + ty.len()..].trim().trim_matches('"');
                // Remove trailing "%1" or " -- %1" etc.
                let val = val.split('"').next().unwrap_or(val)
                    .split(" -- ").next().unwrap_or(val)
                    .trim().trim_matches('"');
                if !val.is_empty() { return Some(val.to_string()); }
            }
        }
    }
    None
}

fn find_versioned_exe(dir: &str, exe_name: &str) -> Option<String> {
    let dir = std::path::Path::new(dir);
    if !dir.is_dir() { return None; }
    let mut candidates: Vec<std::path::PathBuf> = std::fs::read_dir(dir).ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join(exe_name))
        .filter(|p| p.exists())
        .collect();
    candidates.sort();
    candidates.pop().map(|p| p.to_string_lossy().into_owned())
}

fn detect_opera_exe(local: &str, pf: &str, pf86: &str) -> Option<String> {
    // 1. Per-user launcher (most common)
    let launcher = format!("{}\\Programs\\Opera\\launcher.exe", local);
    if exe_exists(&launcher) { return Some(launcher); }

    // 2. Versioned per-user subfolder
    if let Some(p) = find_versioned_exe(&format!("{}\\Programs\\Opera", local), "opera.exe") {
        return Some(p);
    }

    // 3. System-wide install
    for base in &[pf, pf86] {
        let l = format!("{}\\Opera\\launcher.exe", base);
        if exe_exists(&l) { return Some(l); }
        let e = format!("{}\\Opera\\opera.exe", base);
        if exe_exists(&e) { return Some(e); }
    }

    // 4. Registry: HKCU / HKLM StartMenuInternet
    for key in &[
        r"HKCU\SOFTWARE\Clients\StartMenuInternet\OperaStable\shell\open\command",
        r"HKLM\SOFTWARE\Clients\StartMenuInternet\OperaStable\shell\open\command",
        r"HKCU\SOFTWARE\Clients\StartMenuInternet\Opera\shell\open\command",
        r"HKLM\SOFTWARE\Clients\StartMenuInternet\Opera\shell\open\command",
    ] {
        if let Some(cmd) = reg_query_cmd(key) {
            let exe = cmd.trim().trim_matches('"');
            if exe_exists(exe) { return Some(exe.to_string()); }
        }
    }

    // 5. Last resort: HKCU Opera Software key
    if let Some(out) = (|| -> Option<String> {
        #[cfg(windows)] use std::os::windows::process::CommandExt;
        #[allow(unused_mut)]
        let mut cmd = std::process::Command::new("reg");
        cmd.args(["query", r"HKCU\SOFTWARE\Opera Software", "/v", "Last Install dir"]);
        #[cfg(windows)] cmd.creation_flags(0x0800_0000);
        let o = cmd.output().ok()?;
        let s = String::from_utf8_lossy(&o.stdout);
        for line in s.lines() {
            if line.contains("Last Install dir") {
                let dir = line.split_whitespace().last()?;
                let l = format!("{}\\launcher.exe", dir);
                if exe_exists(&l) { return Some(l); }
                let e = format!("{}\\opera.exe", dir);
                if exe_exists(&e) { return Some(e); }
            }
        }
        None
    })() { return Some(out); }

    None
}

fn detect_browser_exes_list() -> Vec<BrowserExe> {
    let local   = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let pf      = std::env::var("PROGRAMFILES").unwrap_or_default();
    let pf86    = std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| pf.clone());

    let mut result = Vec::new();

    let candidates: &[(&str, &[&str])] = &[
        ("Google Chrome", &[
            &format!("{}\\Google\\Chrome\\Application\\chrome.exe", local) as &str,
            &format!("{}\\Google\\Chrome\\Application\\chrome.exe", pf),
            &format!("{}\\Google\\Chrome\\Application\\chrome.exe", pf86),
        ]),
        ("Microsoft Edge", &[
            &format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", pf),
            &format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", pf86),
            &format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", local),
        ]),
        ("Mozilla Firefox", &[
            &format!("{}\\Mozilla Firefox\\firefox.exe", pf),
            &format!("{}\\Mozilla Firefox\\firefox.exe", pf86),
            &format!("{}\\Programs\\Mozilla Firefox\\firefox.exe", local),
        ]),
        ("Brave", &[
            &format!("{}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", pf),
            &format!("{}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", local),
        ]),
        ("Vivaldi", &[
            &format!("{}\\Vivaldi\\Application\\vivaldi.exe", local),
            &format!("{}\\Vivaldi\\Application\\vivaldi.exe", pf),
        ]),
        ("Opera GX", &[
            &format!("{}\\Programs\\Opera GX\\launcher.exe", local),
            &format!("{}\\Opera GX Stable\\launcher.exe", pf),
        ]),
        ("Waterfox", &[
            &format!("{}\\Waterfox\\waterfox.exe", pf),
            &format!("{}\\Waterfox\\waterfox.exe", pf86),
        ]),
    ];

    for (name, paths) in candidates {
        let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        if let Some(path) = paths.into_iter().find(|p| exe_exists(p)) {
            result.push(BrowserExe { name: name.to_string(), path });
        }
    }

    // Opera: dedicated multi-path + registry detection
    if let Some(path) = detect_opera_exe(&local, &pf, &pf86) {
        // Insert Opera after Edge if not already added
        if !result.iter().any(|b| b.name == "Opera") {
            result.push(BrowserExe { name: "Opera".to_string(), path });
        }
    }

    // Roaming-based Opera (some system installs)
    let opera_roaming = format!("{}\\Opera Software\\Opera Stable\\opera.exe", roaming);
    if exe_exists(&opera_roaming) && !result.iter().any(|b| b.name == "Opera") {
        result.push(BrowserExe { name: "Opera".to_string(), path: opera_roaming });
    }

    result
}

#[tauri::command]
fn detect_browser_exes() -> Vec<BrowserExe> { detect_browser_exes_list() }

// ── Browser config (portable storage) ───────────────────────────────────────

fn browsers_config_path() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("browsers.json"))
}

#[tauri::command]
fn load_browsers_config() -> String {
    read_config_raw("browsers.json")
}

#[tauri::command]
fn save_browsers_config(json: String) -> Result<(), String> {
    let path = browsers_config_path().ok_or_else(|| t!("browsers.no_path"))?;
    std::fs::write(path, json.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
fn detect_browsers() -> Vec<DetectedBrowser> { detect_browsers_list() }

#[tauri::command]
fn import_from_browser(state: tauri::State<AppState>, browser_id: String) -> Result<ImportSummary, String> {
    let browsers = detect_browsers_list();
    logger::log(&format!("импорт из браузера: найдено профилей {}, запрошен «{browser_id}»", browsers.len()));
    let b = browsers.iter().find(|b| b.id == browser_id).ok_or_else(|| {
        logger::log(&format!("импорт: профиль «{browser_id}» не найден среди обнаруженных"));
        t!("import.browser_not_found")
    })?;
    let kind = b.kind.clone();
    let name = b.name.clone();
    let path = b.bookmarks_path.clone();
    drop(browsers);
    logger::log(&format!("импорт «{name}» ({kind}) из {path}"));

    let (links, folders) = if kind == "firefox" {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::import_firefox(&conn, &path, &name).map_err(|e| {
            logger::log(&format!("импорт «{name}» не удался: {e}"));
            e.to_string()
        })?
    } else {
        // Частая причина «ноль закладок» — файл занят браузером (os error 32).
        let json = std::fs::read_to_string(&path).map_err(|e| {
            logger::log(&format!("импорт «{name}»: файл закладок не прочитан ({e})"));
            e.to_string()
        })?;
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::import_chromium(&conn, &json, &name).map_err(|e| {
            logger::log(&format!("импорт «{name}» не удался: {e}"));
            e.to_string()
        })?
    };
    // Ноль — не ошибка, но именно с этим приходят с жалобой: структура файла
    // оказалась не той, что ждём, или все секции закладок пусты.
    if links == 0 && folders == 0 {
        logger::log(&format!("импорт «{name}»: разобрано 0 закладок — структура файла не та или секции пусты"));
    } else {
        logger::log(&format!("импорт «{name}»: {links} ссылок, {folders} папок"));
    }
    Ok(ImportSummary { links, folders })
}

/// Импорт текстового файла — один пункт меню на два формата.
///
/// Формат определяется по содержимому (`db::looks_like_our_txt`), а не
/// спрашивается у человека: спрашивать значило бы требовать от него знать
/// разницу, которую он знать не обязан. Цена ошибки определения снижена
/// счётчиком неразобранных строк — он уходит наверх и показывается.
#[tauri::command]
async fn import_txt_file(
    state: tauri::State<'_, AppState>,
    window: tauri::Window,
    parent_id: Option<i64>,
) -> Result<Option<db::TxtImportResult>, String> {
    // `None` — окно закрыли без выбора.
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.import_txt"))
        .add_filter(t!("rfd.filter.txt"), &["txt"])
        .pick_file().await
    else { return Ok(None) };
    let path = file.path().to_path_buf();
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;

    // Замок берём ПОСЛЕ диалога: MutexGuard через `.await` держать нельзя.
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    if db::looks_like_our_txt(&content) {
        // Наш экспорт: имя корневой папки записано в самом файле, и дерево
        // воссоздаётся вместе с ним — размещать иначе было бы неверно.
        return db::import_txt(&conn, &content, parent_id).map(Some).map_err(|e| e.to_string());
    }

    // Простой список: имя первой строки папкой быть не должно, поэтому имя
    // берётся из имени файла.
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| t!("folder.import"));
    // Пустая целевая папка — кладём прямо в неё, заворачивать содержимое
    // в подпапку там незачем. Непустая папка или импорт из меню, где папка
    // не выбрана, — создаём свою папку.
    let folder = match parent_id {
        Some(id) if db::folder_is_empty(&conn, id) => None,
        _ => Some(
            db::unique_folder_title(&conn, parent_id, &stem)
                .ok_or_else(|| t!("import.too_many_folders"))?,
        ),
    };
    db::import_txt_urls(&conn, &content, folder.as_deref(), parent_id).map(Some).map_err(|e| e.to_string())
}

// ── Import commands ──────────────────────────────────────────────────────────

/// `None` — окно закрыли без выбора; `Some(0)` — файл выбран, нового нет.
#[tauri::command]
async fn import_html(state: tauri::State<'_, AppState>, window: tauri::Window, parent_id: Option<i64>) -> Result<Option<usize>, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.import_html"))
        .add_filter(t!("rfd.filter.html"), &["html", "htm"])
        .pick_file().await
    else { return Ok(None) };
    let content = std::fs::read_to_string(file.path()).map_err(|e| e.to_string())?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::import_html(&conn, &content, parent_id).map(Some).map_err(|e| e.to_string())
}

/// `None` — окно закрыли без выбора; `Some(0)` — файл выбран, нового нет.
#[tauri::command]
async fn import_uadat_pick(state: tauri::State<'_, AppState>, window: tauri::Window, parent_id: Option<i64>) -> Result<Option<usize>, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.import_uadat"))
        .add_filter(t!("rfd.filter.uadat"), &["dat", "bak"])
        .pick_file().await
    else { return Ok(None) };
    let path = file.path().to_path_buf();
    let raw = std::fs::read(&path).map_err(|e| e.to_string())?;
    let (text, _, _) = encoding_rs::WINDOWS_1251.decode(&raw);
    let nodes = importer::parse(&text);
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::import(&conn, &nodes, parent_id).map(Some).map_err(|e| e.to_string())
}

// ── Import from another DB ───────────────────────────────────────────────────

/// `None` — окно закрыли без выбора.
#[tauri::command]
async fn analyze_import_db(
    state: tauri::State<'_, AppState>,
    window: tauri::Window,
) -> Result<Option<db::ImportAnalysis>, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t!("rfd.import_db"))
        .add_filter(t!("rfd.filter.db"), &["db"])
        .pick_file().await
    else { return Ok(None) };
    let src_path = file.path().to_path_buf();
    let src_path_str = src_path.to_string_lossy().to_string();

    let current_path = state.db_path.lock().map_err(|e| e.to_string())?.clone();
    if src_path == current_path {
        return Err(t!("import.same_db"));
    }

    let current_urls = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::collect_urls(&conn).map_err(|e| e.to_string())?
    };

    let nodes = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<db::SrcNode>, String> {
        let src = Connection::open_with_flags(
            &src_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|e| e.to_string())?;
        db::read_src_nodes(&src).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())??;

    Ok(Some(db::analyze_import_db(&nodes, &current_urls, src_path_str)))
}

#[tauri::command]
async fn execute_import_db(
    state: tauri::State<'_, AppState>,
    path: String,
    dest_parent: Option<i64>,
) -> Result<usize, String> {
    let src_path = std::path::PathBuf::from(&path);

    let current_path = state.db_path.lock().map_err(|e| e.to_string())?.clone();
    if src_path == current_path {
        return Err(t!("import.same_db"));
    }

    let nodes = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<db::SrcNode>, String> {
        let src = Connection::open_with_flags(
            &src_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|e| e.to_string())?;
        db::read_src_nodes(&src).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())??;

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let current_urls = db::collect_urls(&conn).map_err(|e| e.to_string())?;
    db::execute_import_from_nodes(&conn, &nodes, dest_parent, &current_urls)
        .map_err(|e| e.to_string())
}

// ── Extension token ──────────────────────────────────────────────────────────

fn gen_token() -> String {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).expect("OS RNG failed");
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

fn load_or_init_token(exe_dir: &std::path::Path) -> String {
    let path = exe_dir.join("settings.json");
    let raw = std::fs::read_to_string(&path).ok();
    // BOM срезаем и здесь: эта функция читает файл сама, мимо read_settings_raw.
    let parsed = raw.as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s.trim_start_matches('\u{feff}')).ok());

    // Файл есть, но не разобрался: ниже он будет ПЕРЕЗАПИСАН одним токеном, и
    // все настройки пользователя исчезнут — прокси, тулбар, панели, хоткей.
    // Сначала откладываем копию и поднимаем флаг для уведомления в UI.
    if raw.is_some() && parsed.is_none() {
        logger::log("settings.json не разобран — файл пересоздаётся, прежний сохраняется как settings.json.bad");
        let _ = backup_bad_settings_file();
        SETTINGS_WERE_RESET.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    let mut v = parsed.unwrap_or_else(|| serde_json::Value::Object(Default::default()));

    if let Some(t) = v.get("extensionToken")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
    {
        return t.to_string();
    }

    let token = gen_token();
    if let serde_json::Value::Object(ref mut map) = v {
        map.insert("extensionToken".into(), serde_json::Value::String(token.clone()));
    }
    if let Ok(json) = serde_json::to_string_pretty(&v) {
        std::fs::write(&path, json).ok();
    }
    token
}

// ── HTTP server ──────────────────────────────────────────────────────────────

const SERVER_PORT: u16 = 27124;
const ALLOWED_ORIGIN: &str = "chrome-extension://imekfalcnffmmmabcjapmihbocjabecf";

fn respond_json(req: tiny_http::Request, status: u16, body: &str, cors: Option<&str>) {
    let mut resp = tiny_http::Response::from_string(body)
        .with_status_code(status)
        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap());
    if let Some(origin) = cors {
        resp = resp.with_header(
            format!("Access-Control-Allow-Origin: {origin}")
                .parse::<tiny_http::Header>().unwrap(),
        );
    }
    req.respond(resp).ok();
}

fn respond_cors_preflight(req: tiny_http::Request, origin: &str) {
    let hdrs = [
        format!("Access-Control-Allow-Origin: {origin}"),
        "Access-Control-Allow-Methods: GET, POST, OPTIONS".to_string(),
        "Access-Control-Allow-Headers: Content-Type, X-UA-Token".to_string(),
        "Access-Control-Max-Age: 86400".to_string(),
    ];
    let mut resp = tiny_http::Response::empty(204);
    for h in hdrs {
        resp = resp.with_header(h.parse::<tiny_http::Header>().unwrap());
    }
    req.respond(resp).ok();
}

fn run_http_server(handle: tauri::AppHandle, token: String, port: u16) {
    let addr = format!("127.0.0.1:{port}");
    let server = match tiny_http::Server::http(&addr) {
        Ok(s)  => s,
        Err(e) => {
            logger::log(&format!("HTTP-сервер расширения не поднялся на {addr}: {e}"));
            return;
        }
    };
    logger::log(&format!("HTTP-сервер расширения слушает {addr}"));

    for mut req in server.incoming_requests() {
        // Validate Origin — only our extension is allowed
        let origin = req.headers().iter()
            .find(|h| h.field.to_string().eq_ignore_ascii_case("origin"))
            .map(|h| h.value.as_str().to_string())
            .unwrap_or_default();
        let cors: Option<&str> = if origin == ALLOWED_ORIGIN { Some(ALLOWED_ORIGIN) } else { None };

        // OPTIONS preflight
        if req.method() == &tiny_http::Method::Options {
            match cors {
                Some(o) => respond_cors_preflight(req, o),
                None    => {
                    logger::log(&format!("расширение: 403 preflight, чужой Origin «{origin}»"));
                    respond_json(req, 403, r#"{"error":"forbidden"}"#, None)
                }
            }
            continue;
        }

        // GET /api/v1/handshake — returns token to the extension (Origin-gated)
        if req.method() == &tiny_http::Method::Post && req.url() == "/api/v1/handshake" {
            match cors {
                None    => {
                    logger::log(&format!("расширение: 403 handshake, чужой Origin «{origin}»"));
                    respond_json(req, 403, r#"{"error":"forbidden"}"#, None)
                }
                Some(o) => respond_json(req, 200,
                    &format!(r#"{{"token":"{}"}}"#, token), Some(o)),
            }
            continue;
        }

        // GET /api/v1/folders — list root folders (Token gated; Origin allowed if absent or matching)
        if req.method() == &tiny_http::Method::Get && req.url() == "/api/v1/folders" {
            if !origin.is_empty() && cors.is_none() {
                logger::log(&format!("расширение: 403 /folders, чужой Origin «{origin}»"));
                respond_json(req, 403, r#"{"error":"forbidden"}"#, None);
                continue;
            }
            let ok = req.headers().iter().any(|h| {
                h.field.to_string().eq_ignore_ascii_case("x-ua-token")
                    && h.value.as_str() == token
            });
            if !ok {
                // Самый частый случай: расширение переустановили или settings.json
                // пересоздан — токены разошлись, и «кнопка в браузере не работает».
                logger::log("расширение: 401 /folders, токен не совпал");
                respond_json(req, 401, r#"{"error":"unauthorized"}"#, Some(ALLOWED_ORIGIN));
                continue;
            }
            let state = handle.state::<AppState>();
            let conn = match state.db.lock() {
                Ok(c)  => c,
                Err(e) => {
                    logger::log(&format!("расширение: 500 /folders, ошибка базы: {e}"));
                    respond_json(req, 500,
                        &format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "\\\"")), Some(ALLOWED_ORIGIN));
                    continue;
                }
            };
            let arr = match extension_folders(&conn) {
                Ok(a)  => a,
                Err(e) => {
                    logger::log(&format!("расширение: 500 /folders, ошибка базы: {e}"));
                    respond_json(req, 500,
                        &format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "\\\"")), Some(ALLOWED_ORIGIN));
                    continue;
                }
            };
            respond_json(req, 200, &arr.to_string(), Some(ALLOWED_ORIGIN));
            continue;
        }

        // All other non-POST or wrong path
        if req.method() != &tiny_http::Method::Post || req.url() != "/api/v1/bookmarks" {
            logger::log(&format!("расширение: 404 {} {}", req.method(), req.url()));
            respond_json(req, 404, r#"{"error":"not found"}"#, None);
            continue;
        }

        // POST /api/v1/bookmarks — Origin check
        if cors.is_none() {
            logger::log(&format!("расширение: 403 /bookmarks, чужой Origin «{origin}»"));
            respond_json(req, 403, r#"{"error":"forbidden"}"#, None);
            continue;
        }

        // Token check
        let ok = req.headers().iter().any(|h| {
            h.field.to_string().eq_ignore_ascii_case("x-ua-token")
                && h.value.as_str() == token
        });
        if !ok {
            logger::log("расширение: 401 /bookmarks, токен не совпал");
            respond_json(req, 401, r#"{"error":"unauthorized"}"#, cors);
            continue;
        }

        let mut body = String::new();
        if req.as_reader().read_to_string(&mut body).is_err() {
            logger::log("расширение: 400 /bookmarks, тело запроса не прочитано");
            respond_json(req, 400, r#"{"error":"bad request"}"#, cors);
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v)  => v,
            Err(e) => {
                logger::log(&format!("расширение: 400 /bookmarks, тело не разобрано: {e}"));
                respond_json(req, 400, r#"{"error":"invalid json"}"#, cors);
                continue;
            }
        };

        let url   = v["url"].as_str().unwrap_or("").trim().to_string();
        let title = v["title"].as_str().unwrap_or("").trim().to_string();
        let note_val: Option<String> = v["note"].as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if url.is_empty() {
            logger::log("расширение: 400 /bookmarks, в запросе нет адреса");
            respond_json(req, 400, r#"{"error":"url required"}"#, cors);
            continue;
        }

        // In "dialog" mode: skip INSERT, ask the app to show the edit dialog
        let add_mode = handle.state::<AppState>()
            .extension_add_mode.lock()
            .map(|m| m.clone())
            .unwrap_or_else(|_| "quick".to_string());
        if add_mode == "dialog" {
            respond_json(req, 200, r#"{"status":"ok"}"#, cors);
            handle.emit("extension-add-request", serde_json::json!({
                "url":       url,
                "title":     title,
                "folder_id": v["folder_id"].as_i64()
            })).ok();
            if let Some(win) = handle.get_webview_window("main") {
                win.unminimize().ok();
                win.set_focus().ok();
            }
            continue;
        }

        // Проверка дубля — только при «не допускать дублей», как у окна «Новая
        // ссылка» (режим «диалог» идёт через него же). Файл читаем до захвата базы.
        let check_dupes = no_duplicate_urls_from_settings();

        // INSERT — conn and state dropped at end of block, before spawn
        let bookmark_id: i64 = {
            let state = handle.state::<AppState>();
            let conn = match state.db.lock() {
                Ok(c)  => c,
                Err(e) => {
                    logger::log(&format!("расширение: 500 /bookmarks, ошибка базы: {e}"));
                    respond_json(req, 500,
                        &format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "\\\"")), cors);
                    continue;
                }
            };
            // Под той же блокировкой, что и INSERT: между проверкой и вставкой
            // вторую такую же ссылку не добавить.
            if check_dupes {
                match db::find_live_bookmark_by_url(&conn, &url) {
                    Ok(Some(hit)) => {
                        logger::log(&format!(
                            "расширение: дубль не добавлен, адрес уже есть (id={}): {url}", hit.id));
                        let body = serde_json::json!({
                            "status": "exists",
                            "id":     hit.id,
                            "folder": hit.folder_title,
                        });
                        respond_json(req, 200, &body.to_string(), cors);
                        continue;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        logger::log(&format!("расширение: 500 /bookmarks, проверка дубля: {e}"));
                        respond_json(req, 500,
                            &format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "\\\"")), cors);
                        continue;
                    }
                }
            }
            let folder_id = {
                let requested = v["folder_id"].as_i64();
                let validated = requested.and_then(|id| {
                    conn.query_row(
                        "SELECT id FROM nodes WHERE id=?1 AND kind='folder'",
                        [id], |r| r.get::<_, i64>(0),
                    ).ok()
                });
                match validated {
                    Some(id) => id,
                    None => match find_or_create_inbox_folder(&conn) {
                        Ok(id) => id,
                        Err(e) => {
                            logger::log(&format!("расширение: 500 /bookmarks, папка не создана: {e}"));
                            respond_json(req, 500,
                                &format!(r#"{{"error":"{}"}}"#, e.replace('"', "\\\"")), cors);
                            continue;
                        }
                    },
                }
            };
            match db::insert_bookmark(&conn, Some(folder_id), &title, &url, note_val.as_deref()) {
                Ok(id) => id,
                Err(e) => {
                    logger::log(&format!("расширение: 500 /bookmarks, ошибка базы: {e}"));
                    respond_json(req, 500,
                        &format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "\\\"")), cors);
                    continue;
                }
            }
        };

        // Respond immediately
        respond_json(req, 200,
            &format!(r#"{{"status":"ok","id":{bookmark_id}}}"#), cors);

        // Notify UI: new bookmark exists (no screenshot yet)
        handle.emit("bookmark-added", serde_json::json!({ "id": bookmark_id })).ok();

        // Fire-and-forget screenshot → emit thumb-updated when done
        let h2   = handle.clone();
        let url2 = normalize_url(&url);
        tauri::async_runtime::spawn(async move {
            let (db_path, data_dir) = {
                let st = h2.state::<AppState>();
                let p  = st.db_path.lock().unwrap();
                (p.clone(), p.parent().unwrap().join("Data"))
            };
            let (old_thumb, opener): (Option<String>, Option<String>) = {
                if let Ok(conn) = h2.state::<AppState>().db.lock() {
                    (conn.query_row("SELECT thumb FROM nodes WHERE id=?1", rusqlite::params![bookmark_id], |r| r.get(0)).ok().flatten(),
                     db::root_opener(&conn, bookmark_id).ok().flatten())
                } else { (None, None) }
            };
            match do_screenshot(data_dir.clone(), bookmark_id, url2, opener, None, None, None).await {
                Ok(shot) => {
                    let path = shot.filename;
                    if let Some(old) = old_thumb {
                        if old != path {
                            let _ = std::fs::remove_file(data_dir.join(&old));
                        }
                    }
                    let st = h2.state::<AppState>();
                    let stored = store_thumb(&st, &db_path, &data_dir, bookmark_id, &path)
                                     .unwrap_or(false);   // причина уже в журнале
                    // Событие только при состоявшейся записи: иначе интерфейс
                    // получит путь к файлу, который мы только что удалили, либо
                    // к снимку чужой базы.
                    if stored {
                        h2.emit("thumb-updated",
                            serde_json::json!({ "id": bookmark_id, "path": path })).ok();
                    }
                }
                Err(e) => logger::log(&format!("расширение: скриншот id={bookmark_id} не создан: {e}")),
            }
        });
    }
}

fn migrate_thumb_to_filename(conn: &Connection, data_dir: &std::path::Path) {
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id, thumb FROM nodes WHERE thumb IS NOT NULL")
        .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.filter_map(|r| r.ok()).collect()))
        .unwrap_or_default();
    for (id, path) in rows {
        if path.contains('/') || path.contains('\\') {
            if let Some(name) = std::path::Path::new(&path).file_name() {
                let name_str = name.to_string_lossy().to_string();
                if data_dir.join(&name_str).exists() {
                    conn.execute(
                        "UPDATE nodes SET thumb = ?1 WHERE id = ?2",
                        rusqlite::params![name_str, id],
                    ).ok();
                }
            }
        }
    }
}

// Общее действие «Добавить из буфера» — вызывается из трея и из хоткея F8.
// Буфер читает JS-listener 'tray-add-from-clipboard' (navigator.clipboard) — payload не нужен.
fn add_from_clipboard(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    app.emit("tray-add-from-clipboard", ()).ok();
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    // Журнал — до всего остального: хук паники ниже пишет только при включённом
    // флаге, а паника может случиться ещё до setup().
    logger::init_from_settings();

    // Паника: в release `panic = "abort"`, раскрутки стека нет — но хук
    // вызывается ДО аборта и записать строку успевает. Без него окно просто
    // исчезает, а журнал обрывается на полуслове, и причины нет нигде.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info.location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "место неизвестно".to_string());   // i18n: не переводится — журнал
        let msg = info.payload().downcast_ref::<&str>().map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "причина неизвестна".to_string());   // i18n: не переводится — журнал
        logger::log(&format!("ПАНИКА в {place}: {msg}"));
        prev_hook(info);   // прежнее поведение не теряем
    }));

    // Язык — до Builder: меню трея строится в setup, а словарь для окна
    // уходит в скрипт инициализации плагином ниже.
    i18n::init();

    tauri::Builder::default()
        // Единственный экземпляр. Плагин ОБЯЗАН идти первым: его setup-хук
        // выполняется раньше остальных и завершает вторую копию до того, как
        // кто-либо успеет открыть ту же базу, занять порт HTTP-сервера
        // расширения или перерегистрировать urlalbum://. Ниже по списку он
        // сработал бы уже после чужих хуков — то есть с половиной последствий.
        // Вторая копия уходит молча: человек нажал ярлык, ему нужно окно,
        // а не сообщение о том, что программа уже запущена.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // «У меня не открывается» нередко означает именно это — свёрнутую
            // в трей первую копию. Аргументы пишем: по ним видно, ярлык это
            // был или ссылка из расширения.
            let args = argv.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            logger::log(&format!("вторая копия отклонена; аргументы: [{args}]"));

            // Ссылка urlalbum:// не должна пропасть вместе со второй копией:
            // отдаём её уже открытому окну тем же событием, каким пользуется
            // расширение (JS слушает его с момента загрузки скрипта).
            if let Some((url, title)) = argv.iter().skip(1)
                .find(|a| a.starts_with("urlalbum://"))
                .and_then(|a| parse_url_scheme(a))
            {
                logger::log(&format!("ссылка из второй копии передана в окно: {url}"));
                app.emit("extension-add-request", serde_json::json!({
                    "url":       url,
                    "title":     title,
                    "folder_id": serde_json::Value::Null,
                })).ok();
            }

            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();          // окно могло быть спрятано в трей
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Словарь — в window.__UA_I18N__ до загрузки app.js (см. i18n::init_script).
        .plugin(tauri::plugin::Builder::<tauri::Wry>::new("ua-i18n")
            .js_init_script(i18n::init_script()).build())
        .setup(|app| {
            // Флаг журнала уже прочитан в main() — здесь только шапка.
            logger::log(&format!("=== запуск URL Album {} ===", env!("CARGO_PKG_VERSION")));
            logger::log(&format!("exe: {}", std::env::current_exe()
                .map(|p| p.display().to_string()).unwrap_or_default()));
            // Логин и пароль прокси в журнал не попадают никогда.
            match proxy_cfg_from_settings() {
                Some(c) => logger::log(&format!("прокси: {}:{}{}",
                    strip_proxy_scheme(&c.host), c.port,
                    if c.user.is_empty() { "" } else { " (с авторизацией)" })),
                None => logger::log("прокси: выключен"),
            }

            // WebView2 рисует всё окно, и «программа не запускается» чаще всего
            // про него: минимум Windows 10 у нас именно из-за него.
            //
            // Строку пишем ВСЕГДА, в том числе когда версию узнать не вышло.
            // Пропуск строки при отказе выглядел бы ровно так же, как её
            // отсутствие в старой сборке, — и вместо диагноза дал бы вопрос
            // «а эта версия программы вообще умеет её писать?». Молчание тут
            // неотличимо от неумения, а «не определён» — уже сам по себе ответ:
            // среда не найдена или повреждена.
            match tauri::webview_version() {
                Ok(v)  => logger::log(&format!("WebView2: {v}")),
                Err(e) => logger::log(&format!("WebView2: не определён ({e})")),
            }
            for note in &i18n::current().notes { logger::log(note); }

            // Профили прошлых снимков, оставшиеся в %TEMP% (упавший снимок,
            // убитая программа). В своём потоке: окно ждать не должно. Час —
            // с запасом: снимок длится не дольше t+8 с.
            let _ = std::thread::Builder::new().name("ua-profile-cleanup".into()).spawn(|| {
                let (removed, failed) = shot_browser::cleanup_stale_profiles(
                    &std::env::temp_dir(), std::time::Duration::from_secs(3600));
                if removed + failed > 0 {
                    logger::log(&format!(
                        "старые профили снимков в %TEMP%: удалено {removed}, не удалось {failed}"));
                }
            });

            // Portable mode: all files live next to the executable.
            let exe_dir = std::env::current_exe()?
                .parent()
                .expect("exe has no parent dir")
                .to_path_buf();

            let token = load_or_init_token(&exe_dir);
            println!("[URL Album] extension token: {token}");

            // Resume last session if the file still exists, else fall back to album.db.
            let db_path = load_last_db()
                .unwrap_or_else(|| exe_dir.join("album.db"));

            // Отказ здесь = окна не будет вообще. Раньше причина не сохранялась
            // нигде: повреждённая база, носитель только для чтения, файл занят —
            // всё выглядело как «программа не запускается».
            let conn = open_startup_db(&db_path)
                .map_err(|e| {
                    logger::log(&e);
                    Box::<dyn std::error::Error>::from(e)
                })?;
            migrate_thumb_to_filename(&conn, &db_path.parent().unwrap_or(&db_path).join("Data"));

            // Persist the resolved path so the next startup knows what was open.
            save_last_db(&db_path);
            logger::log(&format!("открыта база: {}", db_path.display()));

            // Parse urlalbum:// from command line if launched via protocol
            let pending = std::env::args().skip(1)
                .find(|a| a.starts_with("urlalbum://"))
                .and_then(|a| parse_url_scheme(&a));

            app.manage(AppState {
                db:                 Mutex::new(conn),
                db_path:            Mutex::new(db_path),
                pending_open:       Mutex::new(pending),
                extension_add_mode: Mutex::new("quick".to_string()),
                user_hotkey:        Mutex::new(None),
            });

            // urlalbum:// при запуске больше не регистрируется (2.3.3):
            // программа не пишет в реестр, см. register_url_scheme

            // Spawn HTTP server for browser extension
            {
                let handle = app.handle().clone();
                std::thread::Builder::new()
                    .name("http-server".into())
                    .spawn(move || run_http_server(handle, token, SERVER_PORT))
                    .expect("failed to spawn HTTP server thread");
            }

            // ── System tray (v1.0) ───────────────────────────────────────────
            // РИСК 2: иконку вшиваем (include_bytes), НЕ полагаемся на default_window_icon.
            let tray_icon = Image::from_bytes(include_bytes!("../icons/icon.ico"))?;

            let show_i = MenuItem::with_id(app, "show",     t!("tray.show"),     true, None::<&str>)?;
            let add_i  = MenuItem::with_id(app, "add_clip", t!("tray.add_clip"), true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit",     t!("tray.quit"),     true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[
                &show_i, &PredefinedMenuItem::separator(app)?,
                &add_i,  &PredefinedMenuItem::separator(app)?,
                &quit_i,
            ])?;

            let tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .tooltip("URL Album")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)   // меню — только по правой кнопке
                .on_tray_icon_event(|tray, event| {
                    // Левый одиночный клик — переключение окна.
                    // MouseButtonState::Up = клик завершён (нажатие + отпускание).
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            // Прячем ТОЛЬКО если окно и видно, и не свёрнуто, и в фокусе.
                            // Если оно открыто, но перекрыто чужими окнами, человек жмёт
                            // на значок с намерением «покажи» — прятать в этот момент
                            // было бы ровно наоборот.
                            let visible   = w.is_visible().unwrap_or(false);
                            let minimized = w.is_minimized().unwrap_or(false);
                            let focused   = w.is_focused().unwrap_or(false);
                            // Фокус мог отобрать сам клик по значку — тогда is_focused()
                            // уже false, хотя мгновение назад окно было активным.
                            let just_unfocused = LAST_UNFOCUS.lock().ok()
                                .and_then(|g| *g)
                                .map_or(false, |t| t.elapsed() < TRAY_RECENT_UNFOCUS);
                            if visible && !minimized && (focused || just_unfocused) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .on_menu_event(|app, event| {
                    let show_win = |app: &tauri::AppHandle| {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    };
                    match event.id.as_ref() {
                        "show" => show_win(app),
                        // Буфер читаем в JS (navigator.clipboard.readText). Если окажется
                        // ненадёжно (приходит пустым) — переключить на чтение в Rust через
                        // крейт arboard и эмитить app.emit("tray-add-from-clipboard", text)
                        // с уже готовым текстом (JS примет payload, без navigator.clipboard).
                        "add_clip" => add_from_clipboard(app),
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .build(app)?;

            // РИСК 1 (критично): TrayIcon обязан пережить setup, иначе Drop уберёт иконку
            // через секунду после старта. Держим его в managed-состоянии на всё время работы.
            app.manage(tray);

            // Глобальный хоткей. "F8" — единственное место смены комбинации (например на "Ctrl+Alt+A").
            // Хоткей не критичен для запуска: комбинацию мог занять чужой процесс.
            // Раньше здесь стоял `?`, и такой отказ ронял setup целиком — окно
            // не появлялось вовсе, без единого следа.
            if let Err(e) = app.global_shortcut().on_shortcut("F8", |app, _shortcut, event| {
                if event.state() == ShortcutState::Pressed {   // ТОЛЬКО нажатие, не отпускание — иначе сработает дважды
                    add_from_clipboard(app);
                }
            }) {
                logger::log(&format!("F8 не зарегистрирован (занят другой программой?): {e}"));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_tree,
            get_nodes,
            get_bookmarks,
            is_empty,
            find_uadat,
            import_uadat,
            open_url,
            open_url_with,
            check_url,
            test_proxy,
            sort_folder,
            sort_all_bookmarks,
            backup_db,
            backup_db_with_data,
            rename_node,
            set_folder_opener,
            delete_folder,
            export_folder_html,
            export_folder_txt,
            clear_thumb,
            refresh_thumb,
            delete_node,
            get_trash,
            restore_node,
            empty_trash,
            purge_node,
            update_bookmark,
            pick_browser_file,
            update_note,
            search_bookmarks,
            db_stats,
            import_html,
            import_txt_file,
            import_uadat_pick,
            detect_browsers,
            import_from_browser,
            detect_browser_exes,
            load_browsers_config,
            save_browsers_config,
            pick_bookmarks_file,
            pick_profile_folder,
            find_bookmarks_in_folder,
            import_from_bookmarks_file,
            save_text_file,
            clear_screenshots,
            clear_db,
            open_db,
            switch_db,
            move_node,
            set_sort_idx,
            load_settings,
            save_settings,
            list_languages,
            load_toolbar_config,
            save_toolbar_config,
            create_folder,
            create_bookmark,
            inbox_folder,
            create_new_db,
            get_db_path,
            set_window_title,
            hide_window,
            set_log_enabled,
            get_log_path,
            log_from_ui,
            backup_bad_config,
            settings_were_reset,
            checkpoint_db,
            open_file,
            get_data_dir,
            get_thumb_time,
            clipboard_text,
            fetch_favicon,
            update_node_favicon,
            close_db,
            get_recent_dbs,
            get_db_properties,
            get_pending_url,
            set_extension_add_mode,
            set_hotkey,
            analyze_import_db,
            execute_import_db,
        ])
        .on_window_event(|_window, event| {
            // Единственная задача: запомнить момент потери фокуса. Обработчик клика
            // по трею смотрит на него, потому что к моменту доставки события окно
            // уже не в фокусе — его отобрал сам клик по значку.
            if let tauri::WindowEvent::Focused(false) = event {
                if let Ok(mut g) = LAST_UNFOCUS.lock() {
                    *g = Some(std::time::Instant::now());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Единственная точка, где WAL-checkpoint гарантированно выполняется при выходе.
            // Managed-состояние (AppState) при завершении Tauri НЕ дропается, поэтому
            // Connection::drop → sqlite3_close не вызывается и -wal/-shm остаются на диске.
            // app.exit(0) из трея приходит сюда же — отдельный checkpoint там не нужен.
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    if let Ok(conn) = state.db.lock() {
                        // Ошибки игнорируем — выход не блокируем, только пишем в журнал.
                        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)") {
                            logger::log(&format!("выход: WAL-checkpoint не выполнен: {e}"));
                        }
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    // Служебная папка базового языка: тесты идут на вшитом русском словаре.
    const INBOX_FOLDER_NAME: &str = "Новые ссылки";

    #[test]
    fn shot_lock_rejects_second() {
        // Пути только этого теста: список идущих снимков — общий на процесс.
        let a = std::path::Path::new(r"C:\ua_shot_lock_test\Data\1.png");
        let b = std::path::Path::new(r"C:\ua_shot_lock_test\Data\2.png");
        let first = ShotLock::try_acquire(a).expect("первый снимок пускается");
        assert!(ShotLock::try_acquire(a).is_none(), "второй снимок в тот же файл — отказ");
        let other = ShotLock::try_acquire(b).expect("другая ссылка не мешает");
        drop(first);
        assert!(ShotLock::try_acquire(a).is_some(), "первый закончился — снова можно");
        drop(other);
    }

    #[test]
    fn shot_busy_is_code() {
        // Код, а не текст для человека: текст переводится, код — нет.
        assert!(!SHOT_BUSY.is_empty() && SHOT_BUSY.is_ascii(), "SHOT_BUSY — код ASCII: {SHOT_BUSY}");
        // По этому коду пакет в app.js считает ссылку «пропущено» — строки обязаны совпадать.
        let js = include_str!("../../ui/app.js");
        let decl = format!("const SHOT_BUSY = '{SHOT_BUSY}';");
        assert!(js.contains(&decl), "в app.js нет `{decl}`");
    }

    fn inbox_ids(conn: &Connection) -> Vec<i64> {
        let mut st = conn.prepare(
            "SELECT id FROM nodes WHERE kind='folder' AND title=?1 AND parent IS NULL \
             AND COALESCE(deleted, 0) = 0"
        ).unwrap();
        st.query_map([INBOX_FOLDER_NAME], |r| r.get(0)).unwrap()
            .map(|r| r.unwrap()).collect()
    }

    #[test]
    fn new_db_has_inbox() {
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        assert_eq!(inbox_ids(&conn).len(), 1, "в новой базе — одна «{INBOX_FOLDER_NAME}»");
    }

    #[test]
    fn cleared_db_has_inbox() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let f = db::insert_folder(&conn, None, "Папка").unwrap();
        db::insert_bookmark(&conn, Some(f), "x", "https://example.com", None).unwrap();
        clear_nodes(&conn).unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap();
        assert_eq!(total, 1, "после очистки остаётся только служебная папка");
        assert_eq!(inbox_ids(&conn).len(), 1, "в очищенной базе — одна «{INBOX_FOLDER_NAME}»");
    }

    #[test]
    fn inbox_folder_reused_not_duplicated() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let first = find_or_create_inbox_folder(&conn).unwrap();
        let second = find_or_create_inbox_folder(&conn).unwrap();
        assert_eq!(first, second, "вторая просьба отдаёт ту же папку");
        assert_eq!(inbox_ids(&conn), vec![first], "второй «{INBOX_FOLDER_NAME}» не появилось");
    }

    #[test]
    fn inbox_folder_in_trash_not_used() {
        // Удалённая папка лежит в корзине с parent = NULL — с виду корневая.
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let old = find_or_create_inbox_folder(&conn).unwrap();
        conn.execute(
            "UPDATE nodes SET deleted = 1, deleted_parent = parent, parent = NULL WHERE id = ?1",
            [old],
        ).unwrap();
        let id = find_or_create_inbox_folder(&conn).unwrap();
        assert_ne!(id, old, "ссылка не должна уходить в папку из корзины");
        assert_eq!(inbox_ids(&conn), vec![id]);
    }

    /// id узлов с признаком служебной папки.
    fn role_ids(conn: &Connection) -> Vec<i64> {
        let mut st = conn.prepare("SELECT id FROM nodes WHERE role = 'inbox' ORDER BY id").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    #[test]
    fn inbox_has_role() {
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        assert_eq!(role_ids(&conn), inbox_ids(&conn), "новая база: признак у её «{INBOX_FOLDER_NAME}»");
        clear_nodes(&conn).unwrap();
        assert_eq!(role_ids(&conn).len(), 1, "очищенная база: признак у новой папки");
        assert_eq!(role_ids(&conn), inbox_ids(&conn));
    }

    #[test]
    fn inbox_found_by_role_after_rename() {
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        let id = find_or_create_inbox_folder(&conn).unwrap();
        conn.execute("UPDATE nodes SET title = 'Входящие' WHERE id = ?1", [id]).unwrap();
        assert_eq!(find_or_create_inbox_folder(&conn).unwrap(), id, "переименованная — та же служебная");
        let folders: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE kind = 'folder'", [], |r| r.get(0)).unwrap();
        assert_eq!(folders, 1, "вторая папка не заведена");
    }

    #[test]
    fn user_folder_with_inbox_title_not_used() {
        // Папка человека с тем же названием служебной не становится.
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        let id = find_or_create_inbox_folder(&conn).unwrap();
        conn.execute("UPDATE nodes SET title = 'Входящие' WHERE id = ?1", [id]).unwrap();
        db::insert_folder(&conn, None, INBOX_FOLDER_NAME).unwrap();
        assert_eq!(find_or_create_inbox_folder(&conn).unwrap(), id);
        db::init(&conn).unwrap();   // повторное открытие базы признак не переносит
        assert_eq!(role_ids(&conn), vec![id]);
    }

    #[test]
    fn inbox_moved_inside_loses_role() {
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        let old = find_or_create_inbox_folder(&conn).unwrap();
        let p = db::insert_folder(&conn, None, "Папка").unwrap();
        conn.execute("UPDATE nodes SET parent = ?1 WHERE id = ?2", [p, old]).unwrap();
        let new = find_or_create_inbox_folder(&conn).unwrap();
        assert_ne!(new, old, "перенесённая внутрь другой — уже не служебная");
        assert_eq!(role_ids(&conn), vec![new], "признак перешёл к новой, у старой снят");
    }

    #[test]
    fn inbox_restored_from_trash_not_second_role() {
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        let old = find_or_create_inbox_folder(&conn).unwrap();
        conn.execute(
            "UPDATE nodes SET deleted = 1, deleted_parent = parent, parent = NULL WHERE id = ?1",
            [old],
        ).unwrap();
        let new = find_or_create_inbox_folder(&conn).unwrap();
        conn.execute("UPDATE nodes SET deleted = 0 WHERE id = ?1", [old]).unwrap();
        assert_eq!(find_or_create_inbox_folder(&conn).unwrap(), new, "служебная — новая");
        assert_eq!(role_ids(&conn), vec![new]);
    }

    #[test]
    fn inbox_role_is_unique() {
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        assert_eq!(role_ids(&conn).len(), 1, "признак у служебной есть");
        let other = db::insert_folder(&conn, None, "Папка").unwrap();
        assert!(conn.execute("UPDATE nodes SET role = 'inbox' WHERE id = ?1", [other]).is_err(),
                "второй служебной папки база не допускает");
    }

    #[test]
    fn imported_inbox_is_not_service() {
        // Сторож на будущее, на прежнем коде не краснеет: импорт перечисляет
        // колонки в INSERT явно, `role` среди них нет. Ловит правку, которая
        // начнёт копировать узлы целиком, — тогда «Новые ссылки» источника
        // стала бы второй служебной (или импорт упал бы на уникальном индексе).
        let src = Connection::open_in_memory().unwrap();
        init_new_db(&src).unwrap();
        let src_inbox = find_or_create_inbox_folder(&src).unwrap();
        db::insert_bookmark(&src, Some(src_inbox), "из источника", "https://example.org/src", None).unwrap();
        assert_eq!(role_ids(&src), vec![src_inbox], "в источнике у «Новых ссылок» есть признак");

        let dest = Connection::open_in_memory().unwrap();
        init_new_db(&dest).unwrap();
        let own = find_or_create_inbox_folder(&dest).unwrap();

        let nodes = db::read_src_nodes(&src).unwrap();
        let urls = db::collect_urls(&dest).unwrap();
        let added = db::execute_import_from_nodes(&dest, &nodes, None, &urls).unwrap();
        assert_eq!(added, 1, "ссылка источника импортирована");
        let folders: i64 = dest.query_row(
            "SELECT COUNT(*) FROM nodes WHERE kind = 'folder' AND title = ?1",
            [INBOX_FOLDER_NAME], |r| r.get(0)).unwrap();
        assert_eq!(folders, 2, "папка источника пришла вместе со ссылкой");
        assert_eq!(role_ids(&dest), vec![own], "признак — только у своей служебной");
        assert_eq!(find_or_create_inbox_folder(&dest).unwrap(), own);
    }

    #[test]
    fn extension_folders_mark_inbox() {
        // Расширение выбирает служебную по признаку, а не по названию.
        let conn = Connection::open_in_memory().unwrap();
        init_new_db(&conn).unwrap();
        let inbox = find_or_create_inbox_folder(&conn).unwrap();
        conn.execute("UPDATE nodes SET title = 'Входящие' WHERE id = ?1", [inbox]).unwrap();
        let other = db::insert_folder(&conn, None, INBOX_FOLDER_NAME).unwrap();
        let arr = extension_folders(&conn).unwrap();
        let flag = |id: i64| arr.as_array().unwrap().iter()
            .find(|f| f["id"] == id).map(|f| f["inbox"].clone());
        assert_eq!(flag(inbox), Some(serde_json::json!(true)), "служебная — inbox: true");
        assert_eq!(flag(other), Some(serde_json::json!(false)), "папка с тем же названием — inbox: false");
    }

    #[test]
    fn legacy_inbox_marked_on_open() {
        // База от версии без колонки role: таблица старой схемы.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE nodes (id INTEGER PRIMARY KEY AUTOINCREMENT, parent INTEGER,
                 kind TEXT NOT NULL DEFAULT 'bookmark', title TEXT NOT NULL, url TEXT,
                 thumb TEXT, note TEXT, created TEXT, visited TEXT, sort_idx INTEGER DEFAULT 0,
                 favicon TEXT, deleted INTEGER DEFAULT 0, deleted_parent INTEGER, opener TEXT);
             INSERT INTO nodes (id, parent, kind, title, deleted) VALUES
                 (1, NULL, 'folder', 'Новые ссылки', 1),
                 (2, NULL, 'folder', 'Папка', 0),
                 (3, 2,    'folder', 'Новые ссылки', 0),
                 (4, NULL, 'bookmark', 'Новые ссылки', 0),
                 (5, NULL, 'folder', 'Новые ссылки', NULL),
                 (6, NULL, 'folder', 'Новые ссылки', 0);",
        ).unwrap();
        db::init(&conn).unwrap();
        assert_eq!(role_ids(&conn), vec![5],
                   "наименьший id среди живых корневых папок «Новые ссылки» (deleted = NULL — живая)");
        assert_eq!(find_or_create_inbox_folder(&conn).unwrap(), 5);
        db::init(&conn).unwrap();
        assert_eq!(role_ids(&conn), vec![5], "повторное открытие ничего не меняет");
    }

    #[test]
    fn legacy_db_without_inbox_not_marked() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        db::insert_folder(&conn, None, "Папка").unwrap();
        db::init(&conn).unwrap();
        assert!(role_ids(&conn).is_empty(), "служебной нет — признак никому не ставится");
    }

    /// Путь к ещё не созданному файлу базы во временной папке, свой на тест.
    fn startup_db_path(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("ua_startup_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("album.db")
    }

    #[test]
    fn startup_creates_inbox_in_new_db() {
        let path = startup_db_path("new");
        let conn = open_startup_db(&path).unwrap();
        assert_eq!(inbox_ids(&conn).len(), 1, "впервые созданная при старте база — с «{INBOX_FOLDER_NAME}»");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn startup_leaves_existing_empty_db() {
        let path = startup_db_path("existing");
        drop(Connection::open(&path).map(|c| db::init(&c).map(|_| c)).unwrap().unwrap());
        let conn = open_startup_db(&path).unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap();
        assert_eq!(total, 0, "в существующую базу без папок ничего не добавляется");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn thumb_file_name_rejects_paths() {
        assert!(thumb_file_name_ok("123.png"));
        assert!(thumb_file_name_ok("1234567890.png"));
        for bad in ["", ".", "..", "../album.db", "..\\album.db", "a..b.png",
                    "sub/1.png", "sub\\1.png", "/1.png", "\\1.png",
                    "C:1.png", "C:\\x.png", "1.png:stream"] {
            assert!(!thumb_file_name_ok(bad), "пропущено: {bad:?}");
        }
    }
}
