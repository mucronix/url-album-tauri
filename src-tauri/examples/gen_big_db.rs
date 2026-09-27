//! Генератор синтетической базы для замера скорости на больших базах.
//!
//!   cargo run --release --example gen_big_db -- <путь.db> [папок] [ссылок]
//!       [--seed N] [--favicons-dir <папка Data\favicons>]
//!
//! Схема создаётся тем же `db::init`, что и в программе. Существующие базы не
//! трогаются: если файл уже есть — отказ. В релизный exe пример не попадает.
//! Одинаковый seed даёт одинаковую базу — замеры «до» и «после» сравнимы.

#![allow(dead_code)]

#[path = "../src/importer.rs"]
mod importer;
#[path = "../src/db.rs"]
mod db;

use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_FOLDERS: usize = 3_800;
const DEFAULT_LINKS:   usize = 91_000;
const DEFAULT_SEED:    u64   = 20_260_927;
const ROOT_FOLDERS:    usize = 20;
const MAX_DEPTH:       usize = 8;
const DOMAINS:         usize = 300;   // пул доменов = пул файлов favicon
const NOTE_PERCENT:    u64   = 20;
const FAVICON_PERCENT: u64   = 70;
const HOT_PERCENT:     u64   = 30;    // доля ссылок, уходящих в «большие» папки
const HOT_FOLDERS:     usize = 40;

const WORDS: &[&str] = &[
    "Проекты", "Rust", "Музыка", "Рецепты", "Linux", "Фото", "Работа", "Книги",
    "Tauri", "Путешествия", "Документация", "Игры", "Статьи", "Видео", "Разное",
    "Новости", "Инструменты", "Здоровье", "Финансы", "Архив",
];

/// xorshift64* — без новых зависимостей, воспроизводимо по seed.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self { Rng(seed.max(1)) }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize { (self.next() % n as u64) as usize }
    fn percent(&mut self, p: u64) -> bool { self.next() % 100 < p }
    fn word(&mut self) -> &'static str { WORDS[self.below(WORDS.len())] }
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn main() {
    let mut pos: Vec<String> = Vec::new();
    let mut seed = DEFAULT_SEED;
    let mut fav_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seed" => seed = args.next().and_then(|s| s.parse().ok())
                .unwrap_or_else(|| fail("--seed: нужно число")),
            "--favicons-dir" => fav_dir = Some(args.next()
                .unwrap_or_else(|| fail("--favicons-dir: нужен путь")).into()),
            _ if a.starts_with("--") => fail(&format!("Неизвестный параметр: {a}")),
            _ => pos.push(a),
        }
    }
    let Some(db_path) = pos.first().map(PathBuf::from) else {
        fail("Использование: gen_big_db <путь.db> [папок] [ссылок] [--seed N] [--favicons-dir <папка>]");
    };
    let num = |i: usize, def: usize| pos.get(i).map(|s| s.parse::<usize>()
        .unwrap_or_else(|_| fail(&format!("Не число: {s}")))).unwrap_or(def);
    let n_folders = num(1, DEFAULT_FOLDERS).max(1);
    let n_links   = num(2, DEFAULT_LINKS);

    // Все проверки — до создания чего-либо, чтобы отказ не оставлял полдела
    for suffix in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", db_path.display()));
        if p.exists() {
            fail(&format!("Отказ: файл уже существует, существующие базы не трогаю: {}", p.display()));
        }
    }
    if let Some(d) = &fav_dir {
        if !d.is_dir() { fail(&format!("Отказ: папки favicon нет: {}", d.display())); }
    }
    if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| fail(&format!("Не создать папку {}: {e}", parent.display())));
    }

    let t = Instant::now();
    let mut conn = Connection::open(&db_path).unwrap_or_else(|e| fail(&format!("Открытие базы: {e}")));
    db::init(&conn).unwrap_or_else(|e| fail(&format!("db::init: {e}")));
    generate(&mut conn, n_folders, n_links, seed).unwrap_or_else(|e| fail(&format!("Вставка: {e}")));
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    println!("База: {} — {n_folders} папок, {n_links} ссылок, seed {seed}, {:.1} с",
        db_path.display(), t.elapsed().as_secs_f64());

    // Самозамер: то же, что делает команда get_tree, плюс сериализация
    let t = Instant::now();
    let tree = db::get_tree(&conn).unwrap_or_else(|e| fail(&format!("get_tree: {e}")));
    let sql_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let json = serde_json::to_string(&tree).unwrap_or_else(|e| fail(&format!("JSON: {e}")));
    println!("get_tree: {} узлов, SQL {sql_ms} мс, JSON {} мс, {:.1} МБ",
        tree.len(), t.elapsed().as_millis(), json.len() as f64 / 1_048_576.0);
    drop(conn);

    if let Some(d) = fav_dir {
        let (created, skipped) = write_favicons(&d);
        println!("Favicon: создано {created}, уже были {skipped} — {}", d.display());
    }
}

fn generate(conn: &mut Connection, n_folders: usize, n_links: usize, seed: u64) -> rusqlite::Result<()> {
    let mut rng = Rng::new(seed);
    let tx = conn.transaction()?;
    let mut next_idx: HashMap<Option<i64>, i64> = HashMap::new();
    let mut idx = |p: Option<i64>| { let e = next_idx.entry(p).or_insert(0); *e += 1; *e - 1 };

    // Папки: первые ROOT_FOLDERS в корне, дальше — случайный родитель среди уже созданных
    let mut folders: Vec<(i64, usize)> = Vec::with_capacity(n_folders);   // (id, глубина)
    {
        let mut ins = tx.prepare("INSERT INTO nodes (parent,kind,title,sort_idx) VALUES(?1,'folder',?2,?3)")?;
        for i in 0..n_folders {
            let (parent, depth) = if i < ROOT_FOLDERS {
                (None, 0)
            } else {
                loop {
                    let (pid, d) = folders[rng.below(folders.len())];
                    if d + 1 < MAX_DEPTH { break (Some(pid), d + 1); }
                }
            };
            let title = format!("{} {}", rng.word(), i + 1);
            ins.execute(params![parent, title, idx(parent)])?;
            folders.push((tx.last_insert_rowid(), depth));
        }
    }

    // Ссылки: часть — в немногие «большие» папки, остальные равномерно
    let hot: Vec<i64> = (0..HOT_FOLDERS.min(folders.len()))
        .map(|_| folders[rng.below(folders.len())].0).collect();
    {
        let mut ins = tx.prepare(
            "INSERT INTO nodes (parent,kind,title,url,note,favicon,sort_idx) VALUES(?1,'bookmark',?2,?3,?4,?5,?6)")?;
        for i in 0..n_links {
            let parent = if rng.percent(HOT_PERCENT) { hot[rng.below(hot.len())] }
                         else { folders[rng.below(folders.len())].0 };
            let dom = rng.below(DOMAINS);
            let title = format!("{} — {} {} (страница {})", rng.word(), rng.word(), rng.word(), i + 1);
            let url = format!("https://synth-{dom:03}.example/{}/{}?id={i}",
                rng.word().to_lowercase(), rng.next() % 100_000);
            let note = rng.percent(NOTE_PERCENT).then(|| if rng.percent(50) {
                format!("Заметка к ссылке {}.", i + 1)
            } else {
                format!("Заметка к ссылке {}.\nВторая строка: {} и {}.\nТретья строка.", i + 1, rng.word(), rng.word())
            });
            let favicon = rng.percent(FAVICON_PERCENT).then(|| format!("synth-{dom:03}.example.png"));
            ins.execute(params![parent, title, url, note, favicon, idx(Some(parent))])?;
        }
    }
    tx.commit()
}

/// Кладёт по файлу на домен; существующие файлы не перезаписывает (create_new).
fn write_favicons(dir: &Path) -> (usize, usize) {
    let (mut created, mut skipped) = (0, 0);
    for d in 0..DOMAINS {
        let p = dir.join(format!("synth-{d:03}.example.png"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(mut f) => {
                let (r, g, b) = ((d * 67 % 200 + 40) as u8, (d * 131 % 200 + 40) as u8, (d * 29 % 200 + 40) as u8);
                f.write_all(&png16(r, g, b)).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())));
                created += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => skipped += 1,
            Err(e) => fail(&format!("{}: {e}", p.display())),
        }
    }
    (created, skipped)
}

/// PNG 16×16 RGB, квадрат с тёмной рамкой. Deflate несжатым блоком — без zlib-крейта.
fn png16(r: u8, g: u8, b: u8) -> Vec<u8> {
    const W: usize = 16;
    let mut raw = Vec::with_capacity(W * (1 + W * 3));
    for y in 0..W {
        raw.push(0); // фильтр строки: None
        for x in 0..W {
            let edge = x == 0 || y == 0 || x == W - 1 || y == W - 1;
            raw.extend(if edge { [r / 2, g / 2, b / 2] } else { [r, g, b] });
        }
    }
    let len = raw.len() as u16;
    let mut z = vec![0x78, 0x01, 0x01];
    z.extend(len.to_le_bytes());
    z.extend((!len).to_le_bytes());
    z.extend(&raw);
    z.extend(adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend((W as u32).to_be_bytes());
    ihdr.extend((W as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]); // 8 бит, RGB
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 { c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }; }
    }
    !c
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut s) = (1u32, 0u32);
    for &b in bytes { a = (a + b as u32) % 65_521; s = (s + a) % 65_521; }
    (s << 16) | a
}
