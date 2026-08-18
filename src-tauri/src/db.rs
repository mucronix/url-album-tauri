use rusqlite::{Connection, Result, params};
use serde::Serialize;

use crate::importer::ParsedNode;

// ── Public data types returned to the frontend ─────────────────────────────

#[derive(Serialize)]
pub struct TreeNode {
    pub id:      i64,
    pub parent:  Option<i64>,
    pub kind:    String,
    pub title:   String,
    pub url:     Option<String>,
    pub thumb:   Option<String>,
    pub favicon: Option<String>,
    pub note:    Option<String>,
    pub created: Option<String>,
    pub visited:  Option<String>,
    pub sort_idx: i64,
    pub count:    i64,
    pub opener:   Option<String>,
}

#[derive(Serialize)]
pub struct Bookmark {
    pub id:     i64,
    pub title:  String,
    pub url:    String,
    pub thumb:  Option<String>,
    pub favicon: Option<String>,
    pub note:   Option<String>,
}

// ── Schema ──────────────────────────────────────────────────────────────────

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous  = FULL;
         PRAGMA wal_checkpoint(PASSIVE);
         CREATE TABLE IF NOT EXISTS nodes (
             id       INTEGER PRIMARY KEY AUTOINCREMENT,
             parent   INTEGER,
             kind     TEXT NOT NULL DEFAULT 'bookmark',
             title    TEXT NOT NULL,
             url      TEXT,
             thumb    TEXT,
             note     TEXT,
             created  TEXT,
             visited  TEXT,
             sort_idx INTEGER DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_parent
             ON nodes (parent, kind, sort_idx);",
    )?;
    // Migration: add favicon column if absent (silent on existing DBs)
    conn.execute("ALTER TABLE nodes ADD COLUMN favicon TEXT", []).ok();
    // Migration: add soft-delete columns if absent
    conn.execute("ALTER TABLE nodes ADD COLUMN deleted INTEGER DEFAULT 0", []).ok();
    conn.execute("ALTER TABLE nodes ADD COLUMN deleted_parent INTEGER", []).ok();
    // Открывать через: обработчик открытия для папки (NULL/ключ браузера/custom:путь)
    conn.execute("ALTER TABLE nodes ADD COLUMN opener TEXT", []).ok();
    Ok(())
}

#[derive(Serialize)]
pub struct SearchResult {
    pub id:     i64,
    pub parent: Option<i64>,
    pub kind:   String,
    pub title:  String,
    pub url:    String,
    pub thumb:  Option<String>,
    pub favicon: Option<String>,
    pub note:   Option<String>,
}

// ── Queries ─────────────────────────────────────────────────────────────────

pub fn is_empty(conn: &Connection) -> bool {
    conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
        == 0
}

pub fn get_tree(conn: &Connection) -> Result<Vec<TreeNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent, kind, title, url, thumb, note, created, visited, favicon, sort_idx,
                CASE WHEN kind = 'folder'
                     THEN (SELECT COUNT(*) FROM nodes b
                           WHERE b.parent = nodes.id AND b.kind = 'bookmark'
                             AND (b.deleted IS NULL OR b.deleted = 0))
                     ELSE 0
                END AS count,
                opener
         FROM nodes
         WHERE (deleted IS NULL OR deleted = 0)
         ORDER BY sort_idx, id",
    )?;
    let result: rusqlite::Result<Vec<TreeNode>> = stmt.query_map([], |row| {
        Ok(TreeNode {
            id:      row.get(0)?,
            parent:  row.get(1)?,
            kind:    row.get(2)?,
            title:   row.get(3)?,
            url:     row.get(4)?,
            thumb:   row.get(5)?,
            note:    row.get(6)?,
            created: row.get(7)?,
            visited: row.get(8)?,
            favicon:  row.get(9)?,
            sort_idx: row.get(10)?,
            count:    row.get(11)?,
            opener:   row.get(12)?,
        })
    })?.collect();
    result
}

pub fn get_trash(conn: &Connection) -> Result<Vec<TreeNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent, kind, title, url, thumb, note, created, visited, favicon, sort_idx,
                CASE WHEN kind = 'folder'
                     THEN (SELECT COUNT(*) FROM nodes b
                           WHERE b.parent = nodes.id AND b.kind = 'bookmark'
                             AND b.deleted = 1)
                     ELSE 0
                END AS count,
                opener
         FROM nodes
         WHERE deleted = 1
         ORDER BY sort_idx, id",
    )?;
    let result: rusqlite::Result<Vec<TreeNode>> = stmt.query_map([], |row| {
        Ok(TreeNode {
            id:      row.get(0)?,
            parent:  row.get(1)?,
            kind:    row.get(2)?,
            title:   row.get(3)?,
            url:     row.get(4)?,
            thumb:   row.get(5)?,
            note:    row.get(6)?,
            created: row.get(7)?,
            visited: row.get(8)?,
            favicon:  row.get(9)?,
            sort_idx: row.get(10)?,
            count:    row.get(11)?,
            opener:   row.get(12)?,
        })
    })?.collect();
    result
}

pub fn get_bookmarks(conn: &Connection, folder_id: i64) -> Result<Vec<Bookmark>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, url, thumb, note, favicon
         FROM nodes
         WHERE kind = 'bookmark' AND parent = ?1
         ORDER BY sort_idx, id",
    )?;
    let result: rusqlite::Result<Vec<Bookmark>> = stmt.query_map(params![folder_id], |row| {
        Ok(Bookmark {
            id:     row.get(0)?,
            title:  row.get(1)?,
            url:    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            thumb:  row.get(3)?,
            note:   row.get(4)?,
            favicon: row.get(5)?,
        })
    })?.collect();
    result
}

pub fn search_bookmarks(
    conn: &Connection,
    query: &str,
    by_title: bool,
    by_url: bool,
    by_note: bool,
) -> Result<Vec<SearchResult>> {
    let pattern = format!("%{query}%");

    // Bookmark conditions (title/url/note)
    let mut bm_conds: Vec<&str> = Vec::new();
    if by_title { bm_conds.push("title LIKE ?1"); }
    if by_url   { bm_conds.push("url   LIKE ?1"); }
    if by_note  { bm_conds.push("note  LIKE ?1"); }

    // Always search folders by name; bookmarks by the chosen fields.
    let sql = if bm_conds.is_empty() {
        "SELECT id, parent, kind, title, url, thumb, note, favicon
         FROM nodes
         WHERE kind = 'folder' AND title LIKE ?1
         ORDER BY title".to_string()
    } else {
        format!(
            "SELECT id, parent, kind, title, url, thumb, note, favicon
             FROM nodes
             WHERE (kind = 'folder' AND title LIKE ?1)
                OR (kind = 'bookmark' AND ({bm}))
             ORDER BY CASE kind WHEN 'folder' THEN 0 ELSE 1 END, title",
            bm = bm_conds.join(" OR ")
        )
    };

    let mut stmt = conn.prepare(&sql)?;
    let result: rusqlite::Result<Vec<SearchResult>> = stmt.query_map(
        rusqlite::params![pattern],
        |row| Ok(SearchResult {
            id:     row.get(0)?,
            parent: row.get(1)?,
            kind:   row.get(2)?,
            title:  row.get(3)?,
            url:    row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            thumb:  row.get(5)?,
            note:   row.get(6)?,
            favicon: row.get(7)?,
        }),
    )?.collect();
    result
}

// ── Export helpers ───────────────────────────────────────────────────────────

struct ExportNode {
    id:     i64,
    parent: Option<i64>,
    kind:   String,
    title:  String,
    url:    Option<String>,
    note:   Option<String>,
}

/// Поддерево папки для экспорта. Колонки `thumb` здесь больше нет — её читал
/// только удалённый `export_sync`. При правке списка колонок помнить про сдвиг
/// индексов в `r.get(N)`: на этом уже горел `sort_idx`, и заметка молча
/// заполнялась бы именем файла картинки.
fn get_subtree(conn: &Connection, folder_id: i64) -> Result<Vec<ExportNode>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE sub(id, parent, kind, title, url, note, sort_idx) AS (
             SELECT id, parent, kind, title, url, note, sort_idx
             FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent, n.kind, n.title, n.url, n.note, n.sort_idx
             FROM nodes n JOIN sub s ON n.parent = s.id
         )
         SELECT id, parent, kind, title, url, note FROM sub ORDER BY sort_idx, id",
    )?;
    let result: Result<Vec<ExportNode>> = stmt.query_map(params![folder_id], |r| Ok(ExportNode {
        id:     r.get(0)?,
        parent: r.get(1)?,
        kind:   r.get(2)?,
        title:  r.get(3)?,
        url:    r.get(4)?,
        note:   r.get(5)?,
    }))?.collect();
    result
}

fn html_folder(nodes: &[ExportNode], parent: Option<i64>, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut out = String::new();
    for n in nodes.iter().filter(|n| n.parent == parent) {
        if n.kind == "folder" {
            out.push_str(&format!("{indent}<DT><H3>{}</H3>\n{indent}<DL><p>\n", n.title));
            out.push_str(&html_folder(nodes, Some(n.id), depth + 1));
            out.push_str(&format!("{indent}</DL><p>\n"));
        } else if let Some(ref url) = n.url {
            out.push_str(&format!("{indent}<DT><A HREF=\"{url}\">{}</A>\n", n.title));
        }
    }
    out
}

pub fn export_html(conn: &Connection, folder_id: i64) -> Result<String> {
    let nodes = get_subtree(conn, folder_id)?;
    let root = nodes.iter().find(|n| n.id == folder_id).map(|n| n.title.as_str()).unwrap_or("Bookmarks");
    let body = html_folder(&nodes, Some(folder_id), 1);
    Ok(format!(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n\
         <META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n\
         <TITLE>{root}</TITLE>\n<H1>{root}</H1>\n<DL><p>\n{body}</DL>\n"
    ))
}

fn txt_folder(nodes: &[ExportNode], parent: Option<i64>, depth: usize) -> String {
    let prefix = "  ".repeat(depth);
    let mut out = String::new();
    for n in nodes.iter().filter(|n| n.parent == parent) {
        if n.kind == "folder" {
            out.push_str(&format!("{prefix}[{}]\n", n.title));
            out.push_str(&txt_folder(nodes, Some(n.id), depth + 1));
        } else if let Some(ref url) = n.url {
            out.push_str(&format!("{prefix}{} - {url}\n", n.title));
            if let Some(ref note) = n.note {
                out.push_str(&format!("{prefix}  Заметка: {note}\n"));
            }
        }
    }
    out
}

pub fn export_txt(conn: &Connection, folder_id: i64) -> Result<String> {
    let nodes = get_subtree(conn, folder_id)?;
    let root = nodes.iter().find(|n| n.id == folder_id).map(|n| n.title.as_str()).unwrap_or("");
    let body = txt_folder(&nodes, Some(folder_id), 0);
    Ok(format!("[{root}]\n{body}"))
}

// ── Import ───────────────────────────────────────────────────────────────────

/// Insert all parsed nodes into the database.
/// `data_dir` is the absolute path to the folder containing PNG thumbnails.
pub fn import(conn: &Connection, nodes: &[ParsedNode], _data_dir: &str, dest_parent: Option<i64>) -> Result<usize> {
    conn.execute_batch("BEGIN")?;

    // Stack of (depth, parent_id). Sentinel: depth=-1, dest_parent (None = root).
    let mut stack: Vec<(i64, Option<i64>)> = vec![(-1, dest_parent)];
    let mut count = 0usize;

    for (sort_idx, node) in nodes.iter().enumerate() {
        let d = node.depth as i64;

        // Pop entries that are at the same depth or deeper
        while stack
            .last()
            .map(|(sd, _)| *sd >= d)
            .unwrap_or(false)
        {
            stack.pop();
        }

        let parent_id = stack.last().and_then(|(_, pid)| *pid);

        let thumb = node.thumb.as_deref().map(|name| name.to_string());

        conn.execute(
            "INSERT INTO nodes
                 (parent, kind, title, url, thumb, note, created, visited, sort_idx)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                parent_id,
                if node.is_folder { "folder" } else { "bookmark" },
                node.title,
                node.url,
                thumb,
                node.note,
                node.created,
                node.visited,
                sort_idx as i64,
            ],
        )?;

        let new_id = conn.last_insert_rowid();
        count += 1;

        if node.is_folder {
            stack.push((d, Some(new_id)));
        }
    }

    conn.execute_batch("COMMIT")?;
    Ok(count)
}

// ── HTML / TXT / Sync importers ──────────────────────────────────────────────

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
}

fn extract_h3(line: &str) -> Option<String> {
    let start = line.find("<H3").or_else(|| line.find("<h3"))?;
    let open_end = line[start..].find('>')? + start + 1;
    let close = line.find("</H3>").or_else(|| line.find("</h3>"))?;
    if open_end > close { return None; }
    Some(html_unescape(line[open_end..close].trim()))
}

fn extract_a(line: &str) -> Option<(String, String)> {
    let href_pos = if let Some(p) = line.find("HREF=\"") { p + 6 }
                   else if let Some(p) = line.find("href=\"") { p + 6 }
                   else { return None; };
    let href_end = href_pos + line[href_pos..].find('"')?;
    let url = line[href_pos..href_end].to_string();
    let tag_end = href_end + line[href_end..].find('>')? + 1;
    let close = line.find("</A>").or_else(|| line.find("</a>"))?;
    if tag_end > close { return None; }
    let title = html_unescape(line[tag_end..close].trim());
    Some((url, title))
}

pub fn import_html(conn: &Connection, html: &str, dest_parent: Option<i64>) -> Result<usize> {
    conn.execute_batch("BEGIN")?;
    let mut folder_stack: Vec<Option<i64>> = Vec::new();
    let mut pending_folder: Option<i64> = None;
    let mut sort_counters: std::collections::HashMap<Option<i64>, i64> = std::collections::HashMap::new();
    let mut count = 0usize;

    for line in html.lines() {
        let t = line.trim();
        let tl: String = t.chars().map(|c| c.to_ascii_lowercase()).collect();

        if tl.starts_with("<dl") {
            let parent = pending_folder.take().or_else(|| {
                folder_stack.last().copied().flatten().or(dest_parent)
            });
            folder_stack.push(parent);
        } else if tl.starts_with("</dl") {
            folder_stack.pop();
        } else if tl.contains("<h3") {
            if let Some(title) = extract_h3(t) {
                let parent = folder_stack.last().copied().flatten().or(dest_parent);
                let c = sort_counters.entry(parent).or_insert(0);
                let si = *c; *c += 1;
                conn.execute(
                    "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1, 'folder', ?2, ?3)",
                    params![parent, &title, si],
                )?;
                count += 1;
                pending_folder = Some(conn.last_insert_rowid());
            }
        } else if tl.contains("<a ") && tl.contains("href") {
            if let Some((url, title)) = extract_a(t) {
                let parent = folder_stack.last().copied().flatten().or(dest_parent);
                let c = sort_counters.entry(parent).or_insert(0);
                let si = *c; *c += 1;
                conn.execute(
                    "INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1, 'bookmark', ?2, ?3, ?4)",
                    params![parent, &title, &url, si],
                )?;
                count += 1;
            }
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(count)
}

/// Итог разбора текстового файла. Возвращается наверх целиком: одно число
/// «вставлено N» не отличало «файл разобран полностью» от «половина строк
/// молча пропала» — а это ровно то, чем оборачивается неверно определённый
/// формат.
#[derive(Serialize, Default, Debug)]
pub struct TxtImportResult {
    pub folders: usize,
    pub links:   usize,
    /// Строки, не подошедшие ни под одно правило. Для списка ссылок всегда 0:
    /// там любая непустая строка по определению формата считается адресом.
    pub skipped: usize,
    /// Имя созданной папки-контейнера — только для ветки списка ссылок.
    /// `None`, когда контейнер не создавался (целевая папка была пуста) или
    /// когда разбирался наш экспорт: там имена берутся из самого файла.
    pub folder: Option<String>,
}

/// Наш ли это экспорт (`export_txt`) или простой список ссылок.
///
/// Основной признак — первая непустая строка в квадратных скобках: `export_txt`
/// всегда начинает файл с `[имя корневой папки]`, каким бы имя ни было.
/// Подстраховка на случай файла, у которого первую строку срезали руками:
/// где-то ниже встречается `[…]` или строка заметки.
///
/// ⚠️ Известное ограничение, чинить не будем: голый IPv6 без схемы первой
/// строкой (`[2001:db8::1]`) будет принят за наш формат. В списках ссылок так
/// не пишут — адрес идёт со схемой, а `http://[2001:db8::1]/` начинается
/// с `http`. Усложнять разбор ради этого случая дороже, чем он стоит.
pub fn looks_like_our_txt(text: &str) -> bool {
    if let Some(first) = text.lines().map(str::trim).find(|l| !l.is_empty()) {
        if first.starts_with('[') && first.ends_with(']') { return true; }
    }
    text.lines().map(str::trim)
        .any(|l| l.starts_with("Заметка: ") || (l.starts_with('[') && l.ends_with(']')))
}

/// Свободное имя папки в этом родителе: `ссылки`, `ссылки (2)`, `ссылки (3)`…
/// Нумерация, а не время: два импорта в одну минуту дали бы одинаковое имя,
/// то есть время уникальности не гарантирует, а номер гарантирует.
/// `None` — потолок исчерпан; сказать об этом человеку честнее, чем подбирать
/// до бесконечности.
pub fn unique_folder_title(conn: &Connection, parent: Option<i64>, base: &str) -> Option<String> {
    let taken = |title: &str| -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE parent IS ?1 AND kind='folder' AND title = ?2
             AND (deleted IS NULL OR deleted = 0)",
            params![parent, title], |r| r.get::<_, i64>(0),
        ).unwrap_or(0) > 0
    };
    if !taken(base) { return Some(base.to_string()); }
    (2..=99).map(|n| format!("{base} ({n})")).find(|t| !taken(t))
}

/// Пуста ли папка: ни ссылок, ни подпапок. Корзина не в счёт.
/// Папка, где лежат только подпапки, пустой НЕ считается — это раздел, и
/// высыпать в него ссылки значит менять его назначение без просьбы.
/// При ошибке запроса отвечаем «не пуста»: лишняя подпапка безобиднее, чем
/// ссылки, подсыпанные к чужому содержимому.
pub fn folder_is_empty(conn: &Connection, id: i64) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE parent IS ?1 AND (deleted IS NULL OR deleted = 0)",
        params![id], |r| r.get::<_, i64>(0),
    ).unwrap_or(1) == 0
}

/// ⚠️ Известное ограничение (не чинится в 2.3.2): многострочная заметка.
/// `export_txt` пишет её одной строкой `Заметка: …`, поэтому перенос внутри
/// заметки уходит в файл отдельной строкой БЕЗ отступа. При обратном разборе
/// такая строка либо попадёт в `skipped` (если в ней нет « - »), либо станет
/// мусорной ссылкой и посчитается в `links` — счётчик покажет успех. Хуже
/// того, у неё глубина 0, а разбор на каждой строке выталкивает из стека папки
/// с глубиной ≥ текущей: стек схлопывается до корня, и всё последующее ложится
/// не в свою папку. То есть портится не только сама заметка, но и структура
/// ниже по файлу. Лечится сменой формата выгрузки (экранирование переноса либо
/// продолжение с отступом) — это правка обеих сторон, отдельной задачей.
pub fn import_txt(conn: &Connection, text: &str, dest_parent: Option<i64>) -> Result<TxtImportResult> {
    conn.execute_batch("BEGIN")?;
    let mut lines = text.lines();
    let mut res = TxtImportResult::default();
    let mut sort_counters: std::collections::HashMap<Option<i64>, i64> = std::collections::HashMap::new();

    // First non-empty line = root folder name
    let first = loop {
        match lines.next() {
            None => { conn.execute_batch("COMMIT")?; return Ok(res); }
            Some(l) if !l.trim().is_empty() => break l,
            _ => {}
        }
    };
    let root_title = {
        let t = first.trim();
        if t.starts_with('[') && t.ends_with(']') { &t[1..t.len()-1] } else { t }
    };
    // Корневая папка выгрузки встаёт В КОНЕЦ целевой, а не с нуля: иначе она
    // вклинивается между уже лежащими там записями (JS сортирует по sort_idx,
    // при равенстве по id). Тот же дефект чинили в импорте из другой базы.
    let c = sort_counters.entry(dest_parent).or_insert_with(|| next_sort_idx(conn, dest_parent));
    let rsi = *c; *c += 1;
    conn.execute(
        "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1, 'folder', ?2, ?3)",
        params![dest_parent, root_title, rsi],
    )?;
    res.folders += 1;
    let root_id = conn.last_insert_rowid();

    // Stack: (depth, folder_id). Sentinel depth=-1.
    let mut stack: Vec<(i64, i64)> = vec![(-1, root_id)];
    let mut last_bm_id: Option<i64> = None;

    for line in lines {
        if line.trim().is_empty() { continue; }
        let leading = line.len() - line.trim_start().len();
        let depth = (leading / 2) as i64;
        let trimmed = line.trim();

        // Вид строки определяется ДО того, как трогается стек: разделитель
        // ищется только там, где строка не папка и не заметка.
        let is_folder = trimmed.starts_with('[') && trimmed.ends_with(']');
        let is_note   = trimmed.starts_with("Заметка: ");
        let sep       = if is_folder || is_note { None } else { trimmed.rfind(" - ") };

        // Стек выталкивают ТОЛЬКО структурные строки — папка и ссылка.
        // Раньше это делала каждая непустая строка, до разбора: любая строка,
        // которую парсер не признал своей, сбрасывала контекст папок, и всё
        // последующее ложилось в корень. Хвост многострочной заметки — лишь
        // один из способов такую строку получить; случайный мусор в середине
        // файла ломал дерево точно так же, а по счётчику `skipped` отличить
        // безобидный мусор от сбежавшей ветки нельзя — цифра одна и та же.
        if is_folder || sep.is_some() {
            while stack.len() > 1 && stack.last().map(|(d, _)| *d >= depth).unwrap_or(false) {
                stack.pop();
            }
        }
        let parent_id = stack.last().map(|(_, id)| *id).unwrap_or(root_id);

        if is_folder {
            let title = &trimmed[1..trimmed.len()-1];
            let c = sort_counters.entry(Some(parent_id)).or_insert(0);
            let si = *c; *c += 1;
            conn.execute(
                "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1, 'folder', ?2, ?3)",
                params![parent_id, title, si],
            )?;
            res.folders += 1;
            stack.push((depth, conn.last_insert_rowid()));
            last_bm_id = None;
        } else if let Some(note) = trimmed.strip_prefix("Заметка: ") {
            if let Some(bid) = last_bm_id {
                conn.execute("UPDATE nodes SET note = ?1 WHERE id = ?2", params![note, bid])?;
            } else {
                // Заметка раньше первой ссылки — приписать её некому.
                res.skipped += 1;
            }
        } else if let Some(sep) = sep {
            let title = trimmed[..sep].trim();
            let url = trimmed[sep + 3..].trim();
            let c = sort_counters.entry(Some(parent_id)).or_insert(0);
            let si = *c; *c += 1;
            conn.execute(
                "INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1, 'bookmark', ?2, ?3, ?4)",
                params![parent_id, title, url, si],
            )?;
            res.links += 1;
            last_bm_id = Some(conn.last_insert_rowid());
        } else {
            // Финального `else` тут не было, и строка, не похожая ни на папку,
            // ни на заметку, ни на «Название - URL», исчезала бесследно: возврат
            // был одним числом, и «файл разобран» не отличалось от «половина
            // пропала». Теперь пропуски считаются и показываются человеку.
            res.skipped += 1;
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(res)
}

// ── Browser / URL-list importers ─────────────────────────────────────────────

fn to_rq<E: std::fmt::Display>(e: E) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(e.to_string())
}

fn chromium_node(
    conn: &Connection,
    node: &serde_json::Value,
    parent: Option<i64>,
    sort: &mut i64,
    links: &mut usize,
    folders: &mut usize,
) -> Result<()> {
    match node["type"].as_str() {
        Some("folder") => {
            let name = node["name"].as_str().unwrap_or("Папка");
            conn.execute(
                "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,'folder',?2,?3)",
                params![parent, name, *sort],
            )?;
            *sort += 1; *folders += 1;
            let fid = conn.last_insert_rowid();
            if let Some(children) = node["children"].as_array() {
                let mut cs = 0i64;
                for child in children {
                    chromium_node(conn, child, Some(fid), &mut cs, links, folders)?;
                }
            }
        }
        Some("url") => {
            let name = node["name"].as_str().unwrap_or("");
            let url  = node["url"].as_str().unwrap_or("");
            if !url.is_empty() {
                conn.execute(
                    "INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1,'bookmark',?2,?3,?4)",
                    params![parent, name, url, *sort],
                )?;
                *sort += 1; *links += 1;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn import_chromium(conn: &Connection, json: &str, browser_name: &str) -> Result<(usize, usize)> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(to_rq)?;
    conn.execute_batch("BEGIN")?;
    conn.execute(
        "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (NULL,'folder',?1,0)",
        params![browser_name],
    )?;
    let root_id = conn.last_insert_rowid();
    let (mut links, mut folders, mut sort) = (0usize, 0usize, 0i64);

    if let Some(roots) = v["roots"].as_object() {
        for key in &["bookmark_bar", "other", "synced"] {
            if let Some(sect) = roots.get(*key) {
                if let Some(ch) = sect["children"].as_array() {
                    if ch.is_empty() { continue; }
                    let name = sect["name"].as_str().unwrap_or(key);
                    conn.execute(
                        "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,'folder',?2,?3)",
                        params![root_id, name, sort],
                    )?;
                    sort += 1; folders += 1;
                    let sid = conn.last_insert_rowid();
                    let mut cs = 0i64;
                    for child in ch {
                        chromium_node(conn, child, Some(sid), &mut cs, &mut links, &mut folders)?;
                    }
                }
            }
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok((links, folders))
}

pub fn import_firefox(conn: &Connection, places_path: &str, browser_name: &str) -> Result<(usize, usize)> {
    let tmp     = std::env::temp_dir().join("ua_ff_tmp.sqlite");
    let tmp_wal = std::env::temp_dir().join("ua_ff_tmp.sqlite-wal");
    let tmp_shm = std::env::temp_dir().join("ua_ff_tmp.sqlite-shm");

    std::fs::copy(places_path, &tmp).map_err(to_rq)?;
    for (ext, dst) in &[("-wal", &tmp_wal), ("-shm", &tmp_shm)] {
        let src = format!("{}{}", places_path, ext);
        if std::path::Path::new(&src).exists() { let _ = std::fs::copy(&src, dst); }
    }

    let ff = Connection::open_with_flags(&tmp, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(to_rq)?;

    let root_ff_id: i64 = ff.query_row(
        "SELECT id FROM moz_bookmarks WHERE parent IS NULL LIMIT 1", [], |r| r.get(0),
    ).unwrap_or(1);

    struct FfNode { id: i64, parent: i64, bk_type: i64, title: Option<String>, fk: Option<i64>, pos: i64 }

    let mut stmt = ff.prepare(
        "SELECT id, COALESCE(parent,0), type, title, fk, COALESCE(position,0) FROM moz_bookmarks ORDER BY parent, position",
    ).map_err(to_rq)?;
    let ff_nodes: Vec<FfNode> = {
        let r: rusqlite::Result<Vec<FfNode>> = stmt.query_map([], |r| Ok(FfNode {
            id: r.get(0)?, parent: r.get(1)?, bk_type: r.get(2)?,
            title: r.get(3)?, fk: r.get(4)?, pos: r.get(5)?,
        }))?.collect();
        r?
    };

    let mut pstmt = ff.prepare("SELECT id, url FROM moz_places").map_err(to_rq)?;
    let places: std::collections::HashMap<i64, String> = {
        let r: rusqlite::Result<Vec<(i64, String)>> =
            pstmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
        r?.into_iter().collect()
    };
    drop(stmt); drop(pstmt); drop(ff);
    let _ = (std::fs::remove_file(&tmp), std::fs::remove_file(&tmp_wal), std::fs::remove_file(&tmp_shm));

    conn.execute_batch("BEGIN")?;
    conn.execute("INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (NULL,'folder',?1,0)", params![browser_name])?;
    let db_root = conn.last_insert_rowid();
    let (mut links, mut folders) = (0usize, 0usize);
    let mut id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    id_map.insert(root_ff_id, db_root);

    for node in &ff_nodes {
        if node.id == root_ff_id { continue; }
        let Some(&parent_db) = id_map.get(&node.parent) else { continue };
        match node.bk_type {
            2 => {
                let title = node.title.as_deref().filter(|t| !t.is_empty()).unwrap_or("Закладки");
                conn.execute("INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,'folder',?2,?3)",
                    params![parent_db, title, node.pos])?;
                id_map.insert(node.id, conn.last_insert_rowid());
                folders += 1;
            }
            1 => {
                if let Some(fk) = node.fk {
                    if let Some(url) = places.get(&fk) {
                        if !url.starts_with("place:") {
                            let title = node.title.as_deref().filter(|t| !t.is_empty()).unwrap_or(url.as_str());
                            conn.execute("INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1,'bookmark',?2,?3,?4)",
                                params![parent_db, title, url, node.pos])?;
                            links += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok((links, folders))
}

/// Простой список ссылок: одна строка — один адрес.
///
/// `folder_name = None` — класть прямо в `dest_parent` (так делаем, когда
/// целевая папка пуста: заворачивать содержимое в подпапку там незачем).
/// `Some(имя)` — создать папку и класть в неё; имя подбирает вызывающий через
/// `unique_folder_title`.
///
/// Пустые строки и `#`-комментарии в `skipped` не попадают: это не «не
/// разобрано», а сознательный пропуск, и раздувать ими цифру значило бы пугать
/// человека на ровном месте.
pub fn import_txt_urls(
    conn: &Connection,
    text: &str,
    folder_name: Option<&str>,
    dest_parent: Option<i64>,
) -> Result<TxtImportResult> {
    conn.execute_batch("BEGIN")?;
    let mut res = TxtImportResult::default();
    // Куда класть ссылки и с какого номера. Жёсткий `sort_idx = 0` у папки был
    // вклиниванием: она вставала перед содержимым непустой целевой папки.
    let (parent, mut si) = match folder_name {
        Some(name) => {
            conn.execute(
                "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,'folder',?2,?3)",
                params![dest_parent, name, next_sort_idx(conn, dest_parent)],
            )?;
            res.folders += 1;
            res.folder = Some(name.to_string());
            (Some(conn.last_insert_rowid()), 0)
        }
        None => (dest_parent, next_sort_idx(conn, dest_parent)),
    };
    for line in text.lines() {
        let url = line.trim();
        if url.is_empty() || url.starts_with('#') { continue; }
        conn.execute("INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1,'bookmark',?2,?3,?4)",
            params![parent, url, url, si])?;
        si += 1;
        res.links += 1;
    }
    conn.execute_batch("COMMIT")?;
    Ok(res)
}

// ── Import from another DB ────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct ImportAnalysis {
    pub source_path:     String,
    pub total_bookmarks: usize,
    pub new_count:       usize,
    pub duplicate_count: usize,
    pub total_folders:   usize,
}

pub struct SrcNode {
    pub id:     i64,
    pub parent: Option<i64>,
    pub kind:   String,
    pub title:  String,
    pub url:    Option<String>,
    pub note:   Option<String>,
    /// Имя файла значка (`домен.расширение`). В отличие от снимка `{id}.png`,
    /// имя favicon осмысленно в любой базе — оно построено из домена, — поэтому
    /// колонку можно переносить как есть, без файловых операций.
    pub favicon: Option<String>,
}

fn normalize_url_for_dedup(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let without_www = without_scheme.strip_prefix("www.").unwrap_or(without_scheme);
    without_www.trim_end_matches('/').to_string()
}

pub fn collect_urls(conn: &Connection) -> Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT url FROM nodes WHERE kind='bookmark' AND url IS NOT NULL AND url != ''"
    )?;
    let result: Result<std::collections::HashSet<String>> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .map(|r| r.map(|u| normalize_url_for_dedup(&u)))
        .collect();
    result
}

/// Узлы источника В ПОРЯДКЕ ПОЛЬЗОВАТЕЛЯ: `ORDER BY sort_idx, id`, как в
/// `get_subtree`. Раньше стояло `ORDER BY id` — порядок создания, — и ручная
/// сортировка источника при импорте терялась целиком.
pub fn read_src_nodes(src: &Connection) -> Result<Vec<SrcNode>> {
    let mut stmt = src.prepare(
        "SELECT id, parent, kind, title, url, note, favicon FROM nodes ORDER BY sort_idx, id"
    )?;
    let result: Result<Vec<SrcNode>> = stmt.query_map([], |r| Ok(SrcNode {
        id:     r.get(0)?,
        parent: r.get(1)?,
        kind:   r.get(2)?,
        title:  r.get(3)?,
        url:    r.get(4)?,
        note:   r.get(5)?,
        // Колонка дописана В КОНЕЦ списка намеренно: индексы 0–5 остаются за
        // прежними полями и не сдвигаются.
        favicon: r.get(6)?,
    }))?.collect();
    result
}

pub fn analyze_import_db(
    nodes: &[SrcNode],
    current_urls: &std::collections::HashSet<String>,
    source_path: String,
) -> ImportAnalysis {
    let total_folders = nodes.iter().filter(|n| n.kind == "folder").count();
    let mut total_bookmarks = 0usize;
    let mut new_count = 0usize;
    let mut duplicate_count = 0usize;

    for n in nodes.iter().filter(|n| n.kind == "bookmark") {
        let Some(url) = &n.url else { continue };
        if url.is_empty() { continue }
        total_bookmarks += 1;
        if current_urls.contains(&normalize_url_for_dedup(url)) {
            duplicate_count += 1;
        } else {
            new_count += 1;
        }
    }

    ImportAnalysis { source_path, total_bookmarks, new_count, duplicate_count, total_folders }
}

/// Следующий свободный `sort_idx` в папке назначения: `MAX + 1`, у пустой — 0.
/// Нужен, чтобы импортированное вставало В КОНЕЦ, а не вклинивалось между
/// существующими записями. `WHERE parent IS ?1`, а не `=`: у корневых узлов
/// parent = NULL, а `= NULL` в SQLite всегда ложь.
fn next_sort_idx(dest: &Connection, parent: Option<i64>) -> i64 {
    dest.query_row(
        "SELECT COALESCE(MAX(sort_idx), -1) + 1 FROM nodes WHERE parent IS ?1",
        params![parent],
        |r| r.get(0),
    ).unwrap_or(0)
}

pub fn execute_import_from_nodes(
    dest: &Connection,
    nodes: &[SrcNode],
    dest_parent: Option<i64>,
    current_urls: &std::collections::HashSet<String>,
) -> Result<usize> {
    let new_bm_ids: std::collections::HashSet<i64> = nodes.iter()
        .filter(|n| n.kind == "bookmark")
        .filter(|n| n.url.as_deref()
            .map(|u| !u.is_empty() && !current_urls.contains(&normalize_url_for_dedup(u)))
            .unwrap_or(false))
        .map(|n| n.id)
        .collect();

    if new_bm_ids.is_empty() { return Ok(0); }

    let node_map: std::collections::HashMap<i64, &SrcNode> =
        nodes.iter().map(|n| (n.id, n)).collect();

    // Collect ancestor folder IDs needed by new bookmarks
    let mut needed_folders: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for &bm_id in &new_bm_ids {
        let mut cur = node_map.get(&bm_id).and_then(|n| n.parent);
        while let Some(pid) = cur {
            if !needed_folders.insert(pid) { break; }
            cur = node_map.get(&pid).and_then(|n| n.parent);
        }
    }

    // Build parent→children map for BFS (None key = attach to dest_parent)
    //
    // Обходим `nodes`, а НЕ `needed_folders`: последнее — HashSet, порядок его
    // итерации не определён и меняется от запуска к запуску, поэтому папки
    // приезжали в случайном порядке — даже не в порядке создания. `nodes`
    // упорядочен по sort_idx источника, значит и папки встанут как у человека.
    let mut children_of: std::collections::HashMap<Option<i64>, Vec<i64>> =
        std::collections::HashMap::new();
    for node in nodes.iter().filter(|n| needed_folders.contains(&n.id)) {
        let key = node.parent.filter(|p| needed_folders.contains(p));
        children_of.entry(key).or_default().push(node.id);
    }

    dest.execute_batch("BEGIN")?;
    let mut id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    // Нумерация своя для каждого родителя и продолжает существующую: общий
    // счётчик с нуля означал, что импортированное вклинивается между старыми
    // записями — JS сортирует по sort_idx, а при равенстве по id (app.js:1577).
    let mut next_idx: std::collections::HashMap<Option<i64>, i64> =
        std::collections::HashMap::new();
    let mut take_idx = |parent: Option<i64>| -> i64 {
        let slot = next_idx.entry(parent).or_insert_with(|| next_sort_idx(dest, parent));
        let v = *slot;
        *slot += 1;
        v
    };

    // BFS: insert folders top-down
    let mut queue: std::collections::VecDeque<i64> =
        children_of.get(&None).cloned().unwrap_or_default().into();
    while let Some(fid) = queue.pop_front() {
        if let Some(node) = node_map.get(&fid) {
            let new_parent = node.parent
                .and_then(|p| id_map.get(&p).copied())
                .or(dest_parent);
            let si = take_idx(new_parent);
            dest.execute(
                "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,?2,?3,?4)",
                params![new_parent, &node.kind, &node.title, si],
            )?;
            id_map.insert(fid, dest.last_insert_rowid());
            if let Some(children) = children_of.get(&Some(fid)) {
                queue.extend(children);
            }
        }
    }

    // Insert new bookmarks
    let mut count = 0usize;
    for node in nodes.iter().filter(|n| new_bm_ids.contains(&n.id)) {
        let new_parent = node.parent
            .and_then(|p| id_map.get(&p).copied())
            .or(dest_parent);
        let si = take_idx(new_parent);
        dest.execute(
            "INSERT INTO nodes (parent, kind, title, url, note, sort_idx, favicon) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![new_parent, &node.kind, &node.title, &node.url, &node.note, si, &node.favicon],
        )?;
        count += 1;
    }

    dest.execute_batch("COMMIT")?;
    Ok(count)
}

// ── Тесты слоя БД ────────────────────────────────────────────────────────────
//
// Проверяют порядок при импорте из другой базы. Тест писался ПОД ТРИ КОНКРЕТНЫХ
// ДЕФЕКТА и на коде до правки падает по каждому из них:
//   1. `read_src_nodes` читал `ORDER BY id` — терялся ручной порядок источника;
//   2. `children_of` строился обходом HashSet — папки приезжали в случайном
//      порядке, разном от запуска к запуску;
//   3. счётчик `sort_idx` начинался с нуля — импортированное вклинивалось между
//      существующими записями приёмника, а не вставало в конец.
#[cfg(test)]
mod tests {
    use super::*;

    fn folder(c: &Connection, parent: Option<i64>, title: &str, si: i64) -> i64 {
        c.execute(
            "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,'folder',?2,?3)",
            params![parent, title, si],
        ).unwrap();
        c.last_insert_rowid()
    }

    fn link(c: &Connection, parent: Option<i64>, title: &str, url: &str, si: i64) -> i64 {
        c.execute(
            "INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1,'bookmark',?2,?3,?4)",
            params![parent, title, url, si],
        ).unwrap();
        c.last_insert_rowid()
    }

    /// Дети родителя в том порядке, в каком их покажет интерфейс: по sort_idx,
    /// при равенстве — по id (ровно так сортирует app.js).
    fn children(c: &Connection, parent: Option<i64>) -> Vec<(String, i64)> {
        let mut s = c.prepare(
            "SELECT title, sort_idx FROM nodes WHERE parent IS ?1 ORDER BY sort_idx, id"
        ).unwrap();
        let rows = s.query_map(params![parent], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap().filter_map(|x| x.ok()).collect();
        rows
    }

    fn titles(rows: &[(String, i64)]) -> Vec<&str> {
        rows.iter().map(|(t, _)| t.as_str()).collect()
    }

    fn id_of(c: &Connection, title: &str) -> i64 {
        c.query_row("SELECT id FROM nodes WHERE title=?1", params![title], |r| r.get(0)).unwrap()
    }

    fn dump(c: &Connection, parent: Option<i64>, label: &str) {
        println!("--- {label} ---");
        for (t, si) in children(c, parent) {
            println!("    [{si:>2}] {t}");
        }
    }

    /// Источник: всё расставлено ВРУЧНУЮ, и ручной порядок нарочно не совпадает
    /// ни с порядком создания (id), ни с алфавитом.
    fn make_src() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();

        // Корень: создаём Гамма → Альфа → Бета, а показывать надо Альфа, Бета, Гамма.
        let gamma = folder(&c, None, "Гамма", 2);
        let alpha = folder(&c, None, "Альфа", 0);
        let beta  = folder(&c, None, "Бета",  1);

        // Шесть подпапок подряд — на них ловится недетерминированность HashSet.
        // Создаются вперемешку, ручной порядок П1..П6.
        for (n, si) in [(4, 3), (1, 0), (6, 5), (2, 1), (5, 4), (3, 2)] {
            let fid = folder(&c, Some(alpha), &format!("П{n}"), si);
            // Без своей ссылки папка не «нужна» импорту и не поедет вовсе.
            link(&c, Some(fid), &format!("сс-П{n}"), &format!("https://example.com/p{n}"), 0);
        }

        // Ссылки прямо в «Альфа»: создаём сБ → сА, показывать надо сА, сБ.
        link(&c, Some(alpha), "сБ", "https://example.com/b", 7);
        link(&c, Some(alpha), "сА", "https://example.com/a", 6);

        link(&c, Some(beta),  "сВ", "https://example.com/v", 0);
        link(&c, Some(gamma), "сГ", "https://example.com/g", 0);
        c
    }

    /// Приёмник: свои записи с sort_idx 0,1,2 и в корне, и в папке назначения —
    /// без них дефект «вклинивания» не проявится.
    fn make_dest() -> (Connection, i64) {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        link(&c, None, "Свой-1", "https://own.example/1", 0);
        link(&c, None, "Свой-2", "https://own.example/2", 1);
        link(&c, None, "Свой-3", "https://own.example/3", 2);
        let dest = folder(&c, None, "Приёмник", 3);
        link(&c, Some(dest), "стар-1", "https://old.example/1", 0);
        link(&c, Some(dest), "стар-2", "https://old.example/2", 1);
        link(&c, Some(dest), "стар-3", "https://old.example/3", 2);
        (c, dest)
    }

    fn run_import(dest: &Connection, src: &Connection, dest_parent: Option<i64>) -> usize {
        let nodes = read_src_nodes(src).unwrap();
        let urls  = collect_urls(dest).unwrap();
        execute_import_from_nodes(dest, &nodes, dest_parent, &urls).unwrap()
    }

    #[test]
    fn import_keeps_source_order_and_appends_to_the_end() {
        let src = make_src();
        let (dest, dest_id) = make_dest();

        dump(&dest, Some(dest_id), "приёмник ДО импорта");
        let n = run_import(&dest, &src, Some(dest_id));
        println!("    вставлено ссылок: {n}");
        dump(&dest, Some(dest_id), "приёмник ПОСЛЕ импорта");

        let after = children(&dest, Some(dest_id));
        assert_eq!(
            titles(&after),
            vec!["стар-1", "стар-2", "стар-3", "Альфа", "Бета", "Гамма"],
            "импортированное обязано стоять В КОНЦЕ и в порядке источника"
        );
        assert_eq!(after[3].1, 3, "нумерация обязана продолжать существующую, а не начинаться с нуля");

        let alpha = id_of(&dest, "Альфа");
        dump(&dest, Some(alpha), "внутри импортированной «Альфа»");
        assert_eq!(
            titles(&children(&dest, Some(alpha))),
            vec!["П1", "П2", "П3", "П4", "П5", "П6", "сА", "сБ"],
            "папки в ручном порядке источника и выше ссылок"
        );
    }

    #[test]
    fn import_into_root_continues_root_numbering() {
        let src = make_src();
        let (dest, _) = make_dest();
        run_import(&dest, &src, None);
        dump(&dest, None, "корень приёмника ПОСЛЕ импорта в корень");

        let roots = children(&dest, None);
        assert_eq!(
            titles(&roots),
            vec!["Свой-1", "Свой-2", "Свой-3", "Приёмник", "Альфа", "Бета", "Гамма"],
            "в корне импортированное тоже обязано вставать в конец"
        );
        // Ловит `WHERE parent = ?1` вместо `IS`: у корневых parent = NULL,
        // сравнение через `=` всегда ложно, счётчик начался бы с нуля.
        assert_eq!(roots[4].1, 4, "счётчик корня обязан продолжаться");
    }

    #[test]
    fn folder_order_is_deterministic_across_runs() {
        let mut first: Option<Vec<String>> = None;
        for run in 1..=10 {
            let src = make_src();
            let (dest, dest_id) = make_dest();
            run_import(&dest, &src, Some(dest_id));
            let alpha = id_of(&dest, "Альфа");
            let order: Vec<String> = children(&dest, Some(alpha)).into_iter().map(|(t, _)| t).collect();
            println!("    прогон {run:>2}: {order:?}");
            match &first {
                None => first = Some(order),
                Some(f) => assert_eq!(&order, f, "порядок папок разошёлся на прогоне {run}"),
            }
        }
    }

    /// Сторож сдвига индексов колонок в `get_subtree`. Вместе с форматом
    /// синхронизации из запроса убрана колонка `thumb`, и если забыть
    /// перенумеровать `r.get(N)`, в заметку молча попадёт имя файла картинки —
    /// заметит это только человек, открывший выгруженный файл. Данные взяты по
    /// образцу живой базы: заметка `111222` и снимок `235.png` неразличимы
    /// глазами ровно до момента, когда одно встанет на место другого.
    #[test]
    fn export_keeps_note_and_never_leaks_thumb_name() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let root = folder(&c, None, "Папка", 0);
        let bm = link(&c, Some(root), "Ventoy", "https://ventoy.net/", 0);
        c.execute(
            "UPDATE nodes SET note = ?1, thumb = ?2 WHERE id = ?3",
            params!["здесь будет описание ссылки!", "235.png", bm],
        ).unwrap();

        let txt = export_txt(&c, root).unwrap();
        println!("--- export_txt ---\n{txt}");
        assert!(txt.contains("Заметка: здесь будет описание ссылки!"),
                "заметка потерялась или подменена: {txt}");
        assert!(!txt.contains("235.png"), "в выгрузку утекло имя файла снимка: {txt}");

        let html = export_html(&c, root).unwrap();
        println!("--- export_html ---\n{html}");
        assert!(html.contains("https://ventoy.net/"), "ссылка потерялась: {html}");
        assert!(!html.contains("235.png"), "в HTML утекло имя файла снимка: {html}");
    }

    /// Значок переносится вместе со ссылкой, а соседние поля не съезжают.
    ///
    /// Два разных расширения намеренно: имя favicon — это домен ПЛЮС расширение
    /// из Content-Type ответа, поэтому у разных доменов они разные, и тест на
    /// одном `.png` не поймал бы путаницу «взяли не ту колонку». Третья ссылка
    /// с пустым favicon — тот же класс проверки с другой стороны: пустое
    /// значение обязано доехать пустым, а не подхватить соседнее.
    #[test]
    fn import_carries_favicon_and_keeps_neighbour_fields() {
        let src = Connection::open_in_memory().unwrap();
        init(&src).unwrap();
        let f = folder(&src, None, "Папка", 0);
        let a = link(&src, Some(f), "Гитхаб",  "https://github.com/",  0);
        let b = link(&src, Some(f), "Пример",  "https://example.com/", 1);
        let _ = link(&src, Some(f), "Безнач",  "https://noicon.test/", 2);
        src.execute("UPDATE nodes SET favicon='github.com.ico', note='заметка гитхаба' WHERE id=?1",
                    params![a]).unwrap();
        src.execute("UPDATE nodes SET favicon='example.com.png' WHERE id=?1", params![b]).unwrap();

        let (dest, dest_id) = make_dest();
        run_import(&dest, &src, Some(dest_id));

        let imported_folder = id_of(&dest, "Папка");
        let mut stmt = dest.prepare(
            "SELECT title, url, note, favicon, sort_idx FROM nodes
             WHERE parent IS ?1 ORDER BY sort_idx, id"
        ).unwrap();
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>, i64)> = stmt
            .query_map(params![imported_folder], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            }).unwrap().filter_map(|x| x.ok()).collect();
        for row in &rows { println!("    {row:?}"); }

        // Кортежем целиком: если поля перепутаются местами, отчёт покажет,
        // ЧТО с чем, а не просто «favicon не тот».
        assert_eq!(rows[0], ("Гитхаб".to_string(), Some("https://github.com/".to_string()),
                             Some("заметка гитхаба".to_string()), Some("github.com.ico".to_string()), 0));
        assert_eq!(rows[1], ("Пример".to_string(), Some("https://example.com/".to_string()),
                             None, Some("example.com.png".to_string()), 1));
        assert_eq!(rows[2], ("Безнач".to_string(), Some("https://noicon.test/".to_string()),
                             None, None, 2));
    }

    /// «Пусто» = ни одного дочернего узла. Папка, где лежат только подпапки,
    /// пустой НЕ считается: это раздел, и высыпать в него ссылки значит менять
    /// его назначение без просьбы.
    #[test]
    fn folder_is_empty_counts_subfolders_too() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let empty    = folder(&c, None, "Пустая", 0);
        let with_bm  = folder(&c, None, "Со ссылкой", 1);
        link(&c, Some(with_bm), "ссылка", "https://a.example/", 0);
        let with_sub = folder(&c, None, "Только подпапки", 2);
        folder(&c, Some(with_sub), "Внутренняя", 0);

        assert!(folder_is_empty(&c, empty));
        assert!(!folder_is_empty(&c, with_bm));
        assert!(!folder_is_empty(&c, with_sub), "папка-раздел пустой не считается");
    }

    /// Нумерация, а не время: имя уникально по построению. Время дало бы
    /// одинаковые имена двум импортам в одну минуту.
    #[test]
    fn unique_folder_title_numbers_collisions() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        assert_eq!(unique_folder_title(&c, None, "ссылки").as_deref(), Some("ссылки"));
        folder(&c, None, "ссылки", 0);
        assert_eq!(unique_folder_title(&c, None, "ссылки").as_deref(), Some("ссылки (2)"));
        folder(&c, None, "ссылки (2)", 1);
        assert_eq!(unique_folder_title(&c, None, "ссылки").as_deref(), Some("ссылки (3)"));

        // Коллизия считается в пределах одного родителя.
        let other = folder(&c, None, "Другая", 2);
        assert_eq!(unique_folder_title(&c, Some(other), "ссылки").as_deref(), Some("ссылки"));
    }

    /// Потолок 99: дальше честный отказ, а не бесконечный подбор.
    #[test]
    fn unique_folder_title_gives_up_after_99() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        folder(&c, None, "имя", 0);
        for n in 2..=99 { folder(&c, None, &format!("имя ({n})"), n); }
        assert_eq!(unique_folder_title(&c, None, "имя"), None);
    }

    /// Целевая папка пуста — контейнер не создаётся, ссылки ложатся прямо в неё.
    #[test]
    fn url_list_without_container_goes_into_the_folder_itself() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let target = folder(&c, None, "Пустая", 0);
        let res = import_txt_urls(&c, "https://a.example/\nhttps://b.example/\n", None, Some(target)).unwrap();

        assert_eq!((res.folders, res.links, res.skipped), (0, 2, 0));
        assert!(res.folder.is_none(), "контейнера быть не должно");
        assert_eq!(titles(&children(&c, Some(target))),
                   vec!["https://a.example/", "https://b.example/"]);
    }

    /// КРАСНЫЙ на коде до правки: контейнер вставлялся с `sort_idx = 0`
    /// и вклинивался перед содержимым непустой папки — выходило
    /// ["стар-1", "ссылки", "стар-2", "стар-3"]. Тот же класс, что `531d8aa`.
    #[test]
    fn url_list_container_goes_to_the_end() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let target = folder(&c, None, "Непустая", 0);
        link(&c, Some(target), "стар-1", "https://old.example/1", 0);
        link(&c, Some(target), "стар-2", "https://old.example/2", 1);
        link(&c, Some(target), "стар-3", "https://old.example/3", 2);

        let res = import_txt_urls(&c, "https://a.example/\n", Some("ссылки"), Some(target)).unwrap();
        assert_eq!(res.folder.as_deref(), Some("ссылки"));
        assert_eq!(titles(&children(&c, Some(target))),
                   vec!["стар-1", "стар-2", "стар-3", "ссылки"]);
    }

    /// Импорт «из меню», когда папка не выбрана: контейнер в корне и тоже
    /// в конец, а не поверх существующих корневых записей.
    #[test]
    fn url_list_into_root_appends_container() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        link(&c, None, "своя-1", "https://own.example/1", 0);
        link(&c, None, "своя-2", "https://own.example/2", 1);

        import_txt_urls(&c, "https://a.example/\n", Some("ссылки"), None).unwrap();
        assert_eq!(titles(&children(&c, None)), vec!["своя-1", "своя-2", "ссылки"]);
    }

    /// Раньше возврат был одним числом, и строка, не подошедшая ни под одно
    /// правило, исчезала бесследно: «файл разобран» не отличалось от «половина
    /// пропала». Здесь мусорных строк две, плюс заметка-сирота — она идёт
    /// раньше первой ссылки, приписать её некому, и это отдельная ветка кода.
    #[test]
    fn our_txt_counts_folders_links_and_skipped() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let text = concat!(
            "[Закладки]\n",
            "  Заметка: сирота, до первой ссылки\n",
            "Ventoy - https://ventoy.net/\n",
            "  Заметка: описание\n",
            "просто строка без ничего\n",
            "ещё мусор\n",
        );
        let res = import_txt(&c, text, None).unwrap();

        assert_eq!((res.folders, res.links, res.skipped), (1, 1, 3),
                   "две мусорные строки плюс заметка-сирота");
        assert!(res.folder.is_none(), "у нашего формата контейнера нет — имена из файла");
        // Нормальная заметка при этом доехала: она не считается ни ссылкой,
        // ни пропуском.
        assert_eq!(
            c.query_row("SELECT note FROM nodes WHERE title='Ventoy'", [], |r| r.get::<_, String>(0)).unwrap(),
            "описание");
    }

    /// Строка, которую парсер не признал своей, НЕ должна сбрасывать контекст
    /// папок. Раньше выталкивание стека шло для каждой непустой строки, до
    /// разбора: мусор с нулевым отступом выталкивал все папки, и всё, что шло
    /// ниже, ложилось в корень. Мусор в конце файла добавлен намеренно —
    /// чтобы зелёный не получался просто потому, что ломать после него нечего.
    #[test]
    fn stray_line_keeps_folder_context() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let text = concat!(
            "[Корень]\n",
            "[Папка]\n",
            "  Ссылка1 - https://a.example/\n",
            "случайная строка, которую парсер не знает\n",
            "  Ссылка2 - https://b.example/\n",
            "мусор в самом конце файла\n",
        );
        let res = import_txt(&c, text, None).unwrap();

        let folder_id = id_of(&c, "Папка");
        assert_eq!(titles(&children(&c, Some(folder_id))), vec!["Ссылка1", "Ссылка2"],
                   "обе ссылки обязаны остаться в своей папке");
        assert_eq!(titles(&children(&c, Some(id_of(&c, "Корень")))), vec!["Папка"],
                   "в корне выгрузки только папка, ссылок туда попадать не должно");
        assert_eq!(res.skipped, 2, "обе мусорные строки посчитаны");
    }

    /// Через этот тест проходят обе правки формата. Сейчас (починка стека)
    /// хвост многострочной заметки ещё теряется — он уходит в `skipped`, — но
    /// структура ниже уже не рушится. Когда появится продолжение с отступом,
    /// ожидание `skipped` сменится с 1 на 0: менять надо именно это число,
    /// а не заводить рядом второй тест, иначе в истории не видно перехода.
    #[test]
    fn multiline_note_does_not_break_structure_below() {
        let src = Connection::open_in_memory().unwrap();
        init(&src).unwrap();
        let root = folder(&src, None, "Закладки", 0);
        let work = folder(&src, Some(root), "Работа", 0);
        let a = link(&src, Some(work), "Ventoy", "https://ventoy.net/", 0);
        link(&src, Some(work), "Rust", "https://rust-lang.org/", 1);
        src.execute("UPDATE nodes SET note = ?1 WHERE id = ?2",
                    params!["первая строка\nвторая строка", a]).unwrap();

        let txt = export_txt(&src, root).unwrap();
        let dest = Connection::open_in_memory().unwrap();
        init(&dest).unwrap();
        let res = import_txt(&dest, &txt, None).unwrap();

        assert_eq!(titles(&children(&dest, Some(id_of(&dest, "Работа")))),
                   vec!["Ventoy", "Rust"],
                   "ссылка после многострочной заметки обязана остаться в своей папке");
        assert_eq!(res.skipped, 1, "хвост заметки пока теряется — это чинит второй коммит");
    }

    #[test]
    fn format_detection_both_ways_and_its_known_limit() {
        // Наш экспорт — по первой строке в скобках.
        assert!(looks_like_our_txt("[Закладки]\nVentoy - https://ventoy.net/\n"));
        // Файл, у которого первую строку срезали руками, ловит подстраховка.
        assert!(looks_like_our_txt("Ventoy - https://ventoy.net/\n  Заметка: описание\n"));

        // Список ссылок — не наш формат ни при каких условиях.
        assert!(!looks_like_our_txt("https://a.example/\nhttps://b.example/\n"));
        assert!(!looks_like_our_txt("# комментарий\nhttps://a.example/\n"));
        assert!(!looks_like_our_txt(""));

        // ⚠️ ЗАФИКСИРОВАННОЕ ОГРАНИЧЕНИЕ, а не забытый случай: голый IPv6 без
        // схемы первой строкой принимается за наш формат. В списках так не
        // пишут — адрес идёт со схемой, а `http://[2001:db8::1]/` начинается
        // с `http`. Тест закрепляет поведение, чтобы смена была осознанной.
        assert!(looks_like_our_txt("[2001:db8::1]\nhttps://a.example/\n"));
    }

    /// Круг «`export_txt` → `db::import_txt`»: структура, названия, адреса
    /// и заметки обязаны доехать.
    ///
    /// ⚠️ ЭТОТ ТЕСТ НЕ ДОКАЗЫВАЕТ ПРАВКУ, РАДИ КОТОРОЙ НАПИСАН, и был зелёным
    /// до неё. Дефект 2.3.2 был не в разборе, а в ПОДКЛЮЧЕНИИ: команда
    /// `import_txt` существовала и работала, но ни один пункт меню её не звал —
    /// в интерфейсе висел только разбор списка URL (`import_txt_urls`), который
    /// наш же экспорт превращал в мусор: 7 строк → 7 «закладок», ни одного
    /// настоящего адреса, ноль папок, ноль заметок. Rust-тест до слоя JS
    /// не достаёт, и никакой тест здесь этого поймать не мог.
    ///
    /// Сторожит он другое, и это тоже нужно: **пару форматов**. Поменяет кто-то
    /// `export_txt`, не тронув парсер, — покраснеет здесь, а не у человека,
    /// который через полгода откроет выгруженный файл.
    #[test]
    fn txt_roundtrip_keeps_folders_and_notes() {
        let src = Connection::open_in_memory().unwrap();
        init(&src).unwrap();
        let root = folder(&src, None, "Закладки", 0);
        let work = folder(&src, Some(root), "Работа", 0);
        let a = link(&src, Some(work), "Ventoy", "https://ventoy.net/", 0);
        link(&src, Some(work), "Rust", "https://rust-lang.org/", 1);
        let b = link(&src, Some(root), "Пример", "https://example.com/", 1);
        src.execute("UPDATE nodes SET note='здесь будет описание' WHERE id=?1", params![a]).unwrap();
        src.execute("UPDATE nodes SET note='проверка заметки 12345' WHERE id=?1", params![b]).unwrap();

        let txt = export_txt(&src, root).unwrap();
        println!("--- export_txt ---\n{txt}");

        let dest = Connection::open_in_memory().unwrap();
        init(&dest).unwrap();
        let n = import_txt(&dest, &txt, None).unwrap();
        println!("итог разбора: {n:?}");

        // Корень выгрузки воссоздан как папка.
        let new_root = id_of(&dest, "Закладки");
        assert_eq!(titles(&children(&dest, Some(new_root))), vec!["Работа", "Пример"],
                   "подпапка и ссылка корня должны лежать в воссозданной папке");

        // Подпапка со своими ссылками — вложенность не потерялась.
        let new_work = id_of(&dest, "Работа");
        assert_eq!(titles(&children(&dest, Some(new_work))), vec!["Ventoy", "Rust"]);

        // Адреса и заметки.
        let got = |title: &str| -> (Option<String>, Option<String>) {
            dest.query_row("SELECT url, note FROM nodes WHERE title=?1", params![title],
                           |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
        };
        assert_eq!(got("Ventoy"), (Some("https://ventoy.net/".into()),
                                   Some("здесь будет описание".into())));
        assert_eq!(got("Rust"),   (Some("https://rust-lang.org/".into()), None));
        assert_eq!(got("Пример"), (Some("https://example.com/".into()),
                                   Some("проверка заметки 12345".into())));
    }
}
