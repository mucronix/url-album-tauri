// ── Журнал для диагностики ───────────────────────────────────────────────────
//
// `url-album.log` рядом с exe. Выключен по умолчанию, включается галочкой в
// настройках; пользователь присылает файл при жалобе.
//
// Правила модуля:
// * журнал не имеет права мешать работе программы — все ошибки глушим;
// * выключённый журнал не делает ни одного обращения к диску;
// * пароль прокси и заголовок `Proxy-Authorization` сюда не попадают НИКОГДА
//   (следить на стороне вызывающего: в лог идёт текст, а не структуры).

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// Включён ли журнал. Кэш флага из `settings.json`: читать файл на каждую
/// запись нельзя — при пакете скриншотов это сотни лишних чтений.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Известный размер файла журнала. `u64::MAX` — ещё не выяснен.
///
/// Выясняется один раз за запуск через `metadata()`, дальше к нему просто
/// прибавляется длина каждой строки. Так проверка размера не стоит ни одного
/// обращения к файловой системе, а ротация срабатывает точно на пороге,
/// а не «примерно раз в N строк».
static SIZE: AtomicU64 = AtomicU64::new(u64::MAX);

/// Пишем из разных потоков (релей поднимает поток на соединение, скриншоты
/// живут в spawn_blocking) — строки нельзя чередовать.
static LOCK: Mutex<()> = Mutex::new(());

/// Порог ротации.
const MAX_SIZE: u64 = 2 * 1024 * 1024;

// ── Публичный интерфейс ──────────────────────────────────────────────────────

/// Записать строку в журнал. Если журнал выключен — не делает ничего.
pub fn log(msg: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let _ = write_line(msg);
}

/// Включить или выключить журнал. Зовётся при сохранении настроек, чтобы
/// галочка действовала сразу, без перезапуска программы.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    // Размер могли обнулить снаружи (удалили файл), пока журнал был выключен.
    SIZE.store(u64::MAX, Ordering::Relaxed);
}

/// Прочитать флаг из `settings.json` — один раз при старте программы.
pub fn init_from_settings() {
    set_enabled(read_flag_from_settings());
}

/// Путь к файлу журнала — нужен кнопке «Открыть журнал».
pub fn log_path() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("url-album.log"))
}

// ── Внутреннее ───────────────────────────────────────────────────────────────

/// Отдельное чтение флага: `proxy_cfg_from_settings` тут не при чём, она про
/// прокси. Ключ `logEnabled`, по умолчанию `false`.
fn read_flag_from_settings() -> bool {
    let raw = crate::read_settings_raw();
    if raw.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("logEnabled").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

fn write_line(msg: &str) -> Option<()> {
    let path = log_path()?;
    let line = format!("[{}] {}\n", timestamp(), msg);
    let len  = line.len() as u64;

    let _guard = LOCK.lock().ok()?;

    let mut size = SIZE.load(Ordering::Relaxed);
    if size == u64::MAX {
        size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    }
    if size + len > MAX_SIZE {
        rotate(&path);
        size = 0;
    }

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    f.write_all(line.as_bytes()).ok()?;

    SIZE.store(size + len, Ordering::Relaxed);
    Some(())
}

/// Переименовать журнал в `url-album.log.old`, затерев прежний `.old`.
fn rotate(path: &std::path::Path) {
    let old = path.with_extension("log.old");
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::rename(path, &old);
}

// ── Время ────────────────────────────────────────────────────────────────────

/// `SYSTEMTIME` из WinAPI. Поля и их порядок обязаны совпадать с ABI.
#[repr(C)]
struct SystemTime {
    year:         u16,
    month:        u16,
    day_of_week:  u16,
    day:          u16,
    hour:         u16,
    minute:       u16,
    second:       u16,
    milliseconds: u16,
}

#[link(name = "kernel32")]
extern "system" {
    /// Местное время с учётом часового пояса и перехода на летнее время.
    ///
    /// В std локального времени нет вовсе, а chrono мы не подключаем. kernel32
    /// уже слинкован, так что новых зависимостей это не добавляет. UTC в журнале
    /// не годится: при разборе жалобы «сломалось второго вечером» строка с датой
    /// первого числа сбивает с толку сильнее, чем помогает.
    fn GetLocalTime(lpSystemTime: *mut SystemTime);
}

/// `ГГГГ-ММ-ДД ЧЧ:ММ:СС` по местному времени. Реализация только для Windows —
/// проект и так Windows-only; на другой платформе будет понятная ошибка сборки,
/// а не молча неверное время.
fn timestamp() -> String {
    let mut st = SystemTime {
        year: 0, month: 0, day_of_week: 0, day: 0,
        hour: 0, minute: 0, second: 0, milliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.year, st.month, st.day, st.hour, st.minute, st.second
    )
}
