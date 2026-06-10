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
                           WHERE b.parent = nodes.id AND b.kind = 'bookmark')
                     ELSE 0
                END AS count
         FROM nodes
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
    thumb:  Option<String>,
    note:   Option<String>,
}

fn get_subtree(conn: &Connection, folder_id: i64) -> Result<Vec<ExportNode>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE sub(id, parent, kind, title, url, thumb, note, sort_idx) AS (
             SELECT id, parent, kind, title, url, thumb, note, sort_idx
             FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent, n.kind, n.title, n.url, n.thumb, n.note, n.sort_idx
             FROM nodes n JOIN sub s ON n.parent = s.id
         )
         SELECT id, parent, kind, title, url, thumb, note FROM sub ORDER BY sort_idx, id",
    )?;
    let result: Result<Vec<ExportNode>> = stmt.query_map(params![folder_id], |r| Ok(ExportNode {
        id:     r.get(0)?,
        parent: r.get(1)?,
        kind:   r.get(2)?,
        title:  r.get(3)?,
        url:    r.get(4)?,
        thumb:  r.get(5)?,
        note:   r.get(6)?,
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

pub fn export_sync(conn: &Connection, folder_id: i64, with_images: bool) -> Result<String> {
    let nodes = get_subtree(conn, folder_id)?;
    let mut items = String::new();
    for n in &nodes {
        let thumb = if with_images {
            n.thumb.as_deref().map(|t| format!(",\"thumb\":\"{t}\"")).unwrap_or_default()
        } else {
            String::new()
        };
        let url   = n.url .as_deref().map(|v| format!(",\"url\":\"{v}\""))  .unwrap_or_default();
        let note  = n.note.as_deref().map(|v| format!(",\"note\":\"{v}\"")) .unwrap_or_default();
        let par   = n.parent.map(|p| format!("{p}")).unwrap_or_else(|| "null".into());
        items.push_str(&format!(
            "{{\"id\":{},\"parent\":{par},\"kind\":\"{}\",\"title\":\"{}\"{}{}{}}}",
            n.id, n.kind, n.title.replace('"', "\\\""), url, note, thumb
        ));
        items.push(',');
    }
    items.pop(); // trailing comma
    Ok(format!(
        "{{\"version\":\"1.0\",\"app\":\"url-album\",\"with_images\":{with_images},\"nodes\":[{items}]}}"
    ))
}

// ── Import ───────────────────────────────────────────────────────────────────

/// Insert all parsed nodes into the database.
/// `data_dir` is the absolute path to the folder containing PNG thumbnails.
pub fn import(conn: &Connection, nodes: &[ParsedNode], data_dir: &str, dest_parent: Option<i64>) -> Result<usize> {
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

        // Build absolute thumb path if present
        let thumb = node.thumb.as_deref().map(|name| {
            format!("{}{}{}", data_dir, std::path::MAIN_SEPARATOR, name)
        });

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

pub fn import_txt(conn: &Connection, text: &str, dest_parent: Option<i64>) -> Result<usize> {
    conn.execute_batch("BEGIN")?;
    let mut lines = text.lines();
    let mut count = 0usize;
    let mut sort_counters: std::collections::HashMap<Option<i64>, i64> = std::collections::HashMap::new();

    // First non-empty line = root folder name
    let first = loop {
        match lines.next() {
            None => { conn.execute_batch("COMMIT")?; return Ok(0); }
            Some(l) if !l.trim().is_empty() => break l,
            _ => {}
        }
    };
    let root_title = {
        let t = first.trim();
        if t.starts_with('[') && t.ends_with(']') { &t[1..t.len()-1] } else { t }
    };
    let c = sort_counters.entry(dest_parent).or_insert(0);
    let rsi = *c; *c += 1;
    conn.execute(
        "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1, 'folder', ?2, ?3)",
        params![dest_parent, root_title, rsi],
    )?;
    count += 1;
    let root_id = conn.last_insert_rowid();

    // Stack: (depth, folder_id). Sentinel depth=-1.
    let mut stack: Vec<(i64, i64)> = vec![(-1, root_id)];
    let mut last_bm_id: Option<i64> = None;

    for line in lines {
        if line.trim().is_empty() { continue; }
        let leading = line.len() - line.trim_start().len();
        let depth = (leading / 2) as i64;
        let trimmed = line.trim();

        while stack.len() > 1 && stack.last().map(|(d, _)| *d >= depth).unwrap_or(false) {
            stack.pop();
        }
        let parent_id = stack.last().map(|(_, id)| *id).unwrap_or(root_id);

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let title = &trimmed[1..trimmed.len()-1];
            let c = sort_counters.entry(Some(parent_id)).or_insert(0);
            let si = *c; *c += 1;
            conn.execute(
                "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1, 'folder', ?2, ?3)",
                params![parent_id, title, si],
            )?;
            count += 1;
            stack.push((depth, conn.last_insert_rowid()));
            last_bm_id = None;
        } else if let Some(note) = trimmed.strip_prefix("Заметка: ") {
            if let Some(bid) = last_bm_id {
                conn.execute("UPDATE nodes SET note = ?1 WHERE id = ?2", params![note, bid])?;
            }
        } else if let Some(sep) = trimmed.rfind(" - ") {
            let title = trimmed[..sep].trim();
            let url = trimmed[sep + 3..].trim();
            let c = sort_counters.entry(Some(parent_id)).or_insert(0);
            let si = *c; *c += 1;
            conn.execute(
                "INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1, 'bookmark', ?2, ?3, ?4)",
                params![parent_id, title, url, si],
            )?;
            count += 1;
            last_bm_id = Some(conn.last_insert_rowid());
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(count)
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

pub fn import_txt_urls(conn: &Connection, text: &str, folder_name: &str, dest_parent: Option<i64>) -> Result<usize> {
    conn.execute_batch("BEGIN")?;
    conn.execute("INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,'folder',?2,0)", params![dest_parent, folder_name])?;
    let folder_id = conn.last_insert_rowid();
    let mut count = 0usize;
    for (i, line) in text.lines().enumerate() {
        let url = line.trim();
        if url.is_empty() || url.starts_with('#') { continue; }
        conn.execute("INSERT INTO nodes (parent, kind, title, url, sort_idx) VALUES (?1,'bookmark',?2,?3,?4)",
            params![folder_id, url, url, i as i64])?;
        count += 1;
    }
    conn.execute_batch("COMMIT")?;
    Ok(count)
}

pub struct RawSyncNode {
    pub old_id:     i64,
    pub old_parent: Option<i64>,
    pub kind:       String,
    pub title:      String,
    pub url:        Option<String>,
    pub note:       Option<String>,
}

pub fn import_sync_nodes(
    conn: &Connection,
    nodes: &[RawSyncNode],
    dest_parent: Option<i64>,
) -> Result<usize> {
    conn.execute_batch("BEGIN")?;
    let mut id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut count = 0usize;

    for (sort_idx, node) in nodes.iter().enumerate() {
        let new_parent = node.old_parent
            .and_then(|old| id_map.get(&old).copied())
            .or(dest_parent);
        conn.execute(
            "INSERT INTO nodes (parent, kind, title, url, note, sort_idx) VALUES (?1,?2,?3,?4,?5,?6)",
            params![new_parent, &node.kind, &node.title, &node.url, &node.note, sort_idx as i64],
        )?;
        id_map.insert(node.old_id, conn.last_insert_rowid());
        count += 1;
    }
    conn.execute_batch("COMMIT")?;
    Ok(count)
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

pub fn read_src_nodes(src: &Connection) -> Result<Vec<SrcNode>> {
    let mut stmt = src.prepare(
        "SELECT id, parent, kind, title, url, note FROM nodes ORDER BY id"
    )?;
    let result: Result<Vec<SrcNode>> = stmt.query_map([], |r| Ok(SrcNode {
        id:     r.get(0)?,
        parent: r.get(1)?,
        kind:   r.get(2)?,
        title:  r.get(3)?,
        url:    r.get(4)?,
        note:   r.get(5)?,
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
    let mut children_of: std::collections::HashMap<Option<i64>, Vec<i64>> =
        std::collections::HashMap::new();
    for &fid in &needed_folders {
        if let Some(node) = node_map.get(&fid) {
            let key = node.parent.filter(|p| needed_folders.contains(p));
            children_of.entry(key).or_default().push(fid);
        }
    }

    dest.execute_batch("BEGIN")?;
    let mut id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut sort_idx = 0i64;

    // BFS: insert folders top-down
    let mut queue: std::collections::VecDeque<i64> =
        children_of.get(&None).cloned().unwrap_or_default().into();
    while let Some(fid) = queue.pop_front() {
        if let Some(node) = node_map.get(&fid) {
            let new_parent = node.parent
                .and_then(|p| id_map.get(&p).copied())
                .or(dest_parent);
            dest.execute(
                "INSERT INTO nodes (parent, kind, title, sort_idx) VALUES (?1,?2,?3,?4)",
                params![new_parent, &node.kind, &node.title, sort_idx],
            )?;
            id_map.insert(fid, dest.last_insert_rowid());
            sort_idx += 1;
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
        dest.execute(
            "INSERT INTO nodes (parent, kind, title, url, note, sort_idx) VALUES (?1,?2,?3,?4,?5,?6)",
            params![new_parent, &node.kind, &node.title, &node.url, &node.note, sort_idx],
        )?;
        sort_idx += 1;
        count += 1;
    }

    dest.execute_batch("COMMIT")?;
    Ok(count)
}
