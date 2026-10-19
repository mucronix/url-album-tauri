//! Выбор браузера для снимка сайта (`do_screenshot`).
//!
//! Порядок (решения пользователя, 19.10.2026):
//! 1. «Открывать через…» корневой папки ссылки;
//! 2. браузер по умолчанию из `browsers.json` (`"default"` — браузер Windows);
//! 3. первый умеющий снимать из списка `browsers.json`, в его порядке;
//! 4. встроенный порядок: Edge → Chrome → Opera → Яндекс.
//! Шаги 3–4 — запасные: о них человеку сообщается (`Choice::fallback`).
//!
//! Снимать умеют только браузеры на Chromium (`--headless --screenshot`), и
//! то не все: Brave, Vivaldi (см. `engine_of`) и Opera новее 95 (см.
//! `opera_engine_can_shoot`) не умеют. Firefox умеет, но с другими флагами —
//! отдельная задача.
//!
//! Всё, что трогает диск и реестр, приходит сюда замыканиями: так порядок
//! выбора проверяется тестами без установленных браузеров.

use std::path::{Path, PathBuf};

/// Каким способом браузер снимает.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Engine {
    /// Chrome, Edge, Яндекс: один флаг `--headless=new`.
    Chromium,
    /// Opera / Opera GX: снимает только движок из папки версии, и только до 95.
    Opera,
    /// Firefox, IE, Brave, Vivaldi и всё незнакомое.
    None,
}

/// Имя файла и папка над ним, в нижнем регистре. Разделитель — оба слэша:
/// путь приходит из browsers.json, где его мог вписать человек.
fn file_and_dir(exe: &str) -> (String, String) {
    let lower = exe.to_lowercase();
    let mut parts = lower.rsplit(['\\', '/']);
    let file = parts.next().unwrap_or("").to_string();
    let dir  = parts.next().unwrap_or("").to_string();
    (file, dir)
}

fn is_yandex(exe: &str) -> bool {
    let (file, _) = file_and_dir(exe);
    file == "browser.exe" && exe.to_lowercase().split(['\\', '/']).any(|part| part == "yandex")
}

/// `launcher.exe` — только из папки Opera / Opera GX: имя слишком общее.
fn is_opera(exe: &str) -> bool {
    let (file, dir) = file_and_dir(exe);
    file == "opera.exe" || (file == "launcher.exe" && dir.starts_with("opera"))
}

/// Движок по имени exe (регистр не важен).
pub(crate) fn engine_of(exe: &str) -> Engine {
    if is_opera(exe) { return Engine::Opera; }
    if is_yandex(exe) { return Engine::Chromium; }
    // Brave и Vivaldi — тоже Chromium, но `--screenshot` у них не срабатывает
    // ни с одним флагом (`--headless=new`, `--headless`, `--headless=old`;
    // проверено 19.10.2026 на Brave 149 и Vivaldi 8.2): Vivaldi в headless
    // открывает свой интерфейс и страницу приветствия, а адрес не открывает
    // вовсе; Brave страницу грузит, но обработчик команд не доходит до конца
    // даже на about:blank. Оба висят до убийства — поэтому «не умеют снимать»,
    // и снимает запасной, а не 20 секунд впустую.
    match file_and_dir(exe).0.as_str() {
        "msedge.exe" | "chrome.exe" | "chromium.exe" => Engine::Chromium,
        _ => Engine::None,
    }
}

/// Флаги headless в порядке попыток; у всех одна попытка — повтор удвоил бы
/// время на мёртвой ссылке (решение пользователя).
///
/// Opera 95 (Chromium 109, последняя для Win7) снимает только старым
/// `--headless`; с `--headless=new` она перезапускает себя чужим процессом,
/// снимка не делает и оставляет сирот (проверено в госте Win7, 19.10.2026).
/// Opera 137 не снимает ни с `--headless=new`, ни с `--headless`, ни с
/// `--headless=old` — висит до снятия (Win10, 19.10.2026), поэтому новые Opera
/// сюда не доходят: см. `opera_engine_can_shoot`.
pub(crate) fn headless_flags(engine: Engine) -> &'static [&'static str] {
    match engine {
        Engine::Opera    => &["--headless"],
        Engine::Chromium => &["--headless=new"],
        Engine::None     => &[],
    }
}

/// Человеческое имя браузера для строки состояния и журнала.
pub(crate) fn display_name(exe: &str) -> String {
    if is_opera(exe) {
        return if exe.to_lowercase().contains("opera gx") { "Opera GX" } else { "Opera" }.to_string();
    }
    if is_yandex(exe) { return "Яндекс".to_string(); }
    let known = match file_and_dir(exe).0.as_str() {
        "msedge.exe"   => Some("Edge"),
        "chrome.exe"   => Some("Chrome"),
        "brave.exe"    => Some("Brave"),
        "vivaldi.exe"  => Some("Vivaldi"),
        "chromium.exe" => Some("Chromium"),
        "firefox.exe"  => Some("Firefox"),
        "iexplore.exe" => Some("Internet Explorer"),
        _ => None,
    };
    match known {
        Some(n) => n.to_string(),
        None => Path::new(exe).file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| exe.to_string()),
    }
}

/// «95.0.4635.90» → [95, 0, 4635, 90]; не версия — `None`.
fn parse_version(name: &str) -> Option<Vec<u32>> {
    if !name.contains('.') { return None; }
    name.split('.').map(|p| {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { return None; }
        p.parse().ok()
    }).collect()
}

/// Движок Opera: `opera.exe` из подпапки с наибольшей версией рядом с
/// `launcher.exe` (или корневым `opera.exe`). `None` — движка нет.
///
/// `launcher.exe` и корневой `opera.exe` флаги `--headless`/`--screenshot`
/// не передают дальше — запустится обычное окно, снимка не будет. Поэтому
/// «движка нет» означает «снимать нечем», а не «попробуем лаунчер».
/// (Разведка url-album-3, Opera 95 на Win7.)
pub(crate) fn find_opera_engine(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let is_engine = exe.file_name()
        .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case("opera.exe"))
        && dir.file_name().and_then(|d| parse_version(&d.to_string_lossy())).is_some();
    if is_engine { return Some(exe.to_path_buf()); }

    std::fs::read_dir(dir).ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let ver = parse_version(&e.file_name().to_string_lossy())?;
            let cand = e.path().join("opera.exe");
            cand.is_file().then_some((ver, cand))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, p)| p)
}

/// Последняя версия движка Opera, которая снимает (`--headless --screenshot`).
const OPERA_LAST_SHOOTING: u32 = 95;

/// Снимает ли движок Opera: версия из имени папки над ним не новее 95.
/// Проверены две точки — 95 снимает, 137 нет (см. `headless_flags`); где
/// между ними граница, неизвестно, поэтому «умеет» — только проверенное.
/// Папка не версия — не умеет.
pub(crate) fn opera_engine_can_shoot(engine: &Path) -> bool {
    engine.parent()
        .and_then(|d| d.file_name())
        .and_then(|d| parse_version(&d.to_string_lossy()))
        .and_then(|v| v.first().copied())
        .is_some_and(|major| major <= OPERA_LAST_SHOOTING)
}

/// Разобранный `browsers.json`: `{ browsers: [{name, path}], default }`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BrowsersCfg {
    pub list:    Vec<(String, String)>, // (name, path); path "default" — браузер Windows
    pub default: String,                // путь или "default"
}

/// Разбор как в `loadBrowsersConfig` (app.js): нет `default` или он пуст —
/// `"default"`. Записи без пути пропускаются. Файла нет или он испорчен —
/// `None`: тогда выбор идёт так же, как без списка. Файл здесь только читается.
pub(crate) fn parse_browsers_json(raw: &str) -> Option<BrowsersCfg> {
    let raw = raw.trim_start_matches('\u{feff}');
    if raw.trim().is_empty() { return None; }
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let list = v.get("browsers").and_then(|b| b.as_array()).map(|arr| {
        arr.iter().filter_map(|b| {
            let path = b.get("path")?.as_str()?.trim();
            if path.is_empty() { return None; }
            let name = b.get("name").and_then(|n| n.as_str()).filter(|n| !n.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| display_name(path));
            Some((name, path.to_string()))
        }).collect()
    }).unwrap_or_default();
    let default = v.get("default").and_then(|d| d.as_str()).map(str::trim)
        .filter(|d| !d.is_empty()).unwrap_or("default").to_string();
    Some(BrowsersCfg { list, default })
}

/// Встроенный порядок, если в `browsers.json` ничего не подошло:
/// Edge → Chrome → Opera → Яндекс (решение пользователя; Brave и Vivaldi из
/// него убраны 19.10.2026 — снимать не умеют, см. `engine_of`).
/// Внутри браузера — обычные места установки, включая установку только для
/// пользователя (`%LOCALAPPDATA%`): так стоят Opera и Яндекс по умолчанию.
pub(crate) fn builtin_candidates(local: &str, pf: &str, pf86: &str) -> Vec<String> {
    let mut v = Vec::new();
    for base in [pf86, pf, local] { v.push(format!(r"{base}\Microsoft\Edge\Application\msedge.exe")); }
    for base in [local, pf, pf86] { v.push(format!(r"{base}\Google\Chrome\Application\chrome.exe")); }
    v.push(format!(r"{local}\Programs\Opera\launcher.exe"));
    for base in [pf, pf86] { v.push(format!(r"{base}\Opera\launcher.exe")); }
    v.push(format!(r"{local}\Yandex\YandexBrowser\Application\browser.exe"));
    v
}

/// Итог выбора.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Choice {
    pub exe:      String,          // что запускать (у Opera — движок)
    pub name:     String,          // для человека
    pub fallback: Option<String>,  // Some — сработал запасной, причина
}

/// Что известно о ссылке и настройках.
pub(crate) struct Inputs<'a> {
    pub opener:  Option<&'a str>,
    pub cfg:     Option<&'a BrowsersCfg>,
    pub builtin: &'a [String],
}

/// Выбрать браузер.
/// `system_default` — exe браузера по умолчанию в Windows;
/// `runnable(path)` — что запускать, или почему нечего (`Err` — окончание
/// фразы о браузере: «не найден», «не умеет снимать (…)»).
///
/// `fallback` — причина первой неудачи среди «назначенных» браузеров (папки и
/// по умолчанию), если снимет кто-то после них. Браузер Windows при `"default"`
/// — это и есть назначенный, не запасной.
pub(crate) fn choose(
    inp: &Inputs,
    system_default: &dyn Fn() -> Option<String>,
    runnable: &dyn Fn(&str) -> Result<String, String>,
) -> Option<Choice> {
    let mut miss: Option<String> = None;

    // Ok — снимаем им; Err(причина) — нет. `"default"` — браузер Windows.
    let attempt = |who: &str, path: &str| -> Result<Choice, String> {
        let path = if path == "default" {
            match system_default() {
                Some(p) => p,
                None => return Err(format!("{who} — браузер Windows не определён")),
            }
        } else { path.to_string() };
        let name = display_name(&path);
        if engine_of(&path) == Engine::None {
            return Err(format!("{who} ({name}) не умеет снимать"));
        }
        runnable(&path)
            .map(|exe| Choice { exe, name: name.clone(), fallback: None })
            .map_err(|why| format!("{who} ({name}) {why}"))
    };

    // 1–2. Назначенные: браузер корневой папки, затем браузер по умолчанию.
    let default = inp.cfg.map_or("default", |c| c.default.as_str());
    let opener  = inp.opener.map(str::trim).filter(|o| !o.is_empty());
    let assigned = opener.map(|o| ("браузер папки", o)).into_iter()
        .chain(std::iter::once(("браузер по умолчанию", default)));
    for (who, path) in assigned {
        match attempt(who, path) {
            Ok(mut c) => { c.fallback = miss; return Some(c); }
            Err(why)  => { miss.get_or_insert(why); }
        }
    }

    // 3–4. Запасные: список программы в его порядке, затем встроенный.
    let list = inp.cfg.map(|c| c.list.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    for path in list.into_iter().chain(inp.builtin.iter().map(String::as_str)) {
        if let Ok(mut c) = attempt("", path) {
            c.fallback = miss;
            return Some(c);
        }
    }
    None
}

// ── Система: реестр, диск, процессы ─────────────────────────────────────────

/// Путь к exe из команды запуска (`shell\open\command`):
/// `"C:\…\launcher.exe" -- "%1"`, `C:\…\iexplore.exe %1`.
pub(crate) fn exe_from_command(cmd: &str) -> Option<String> {
    let cmd = cmd.trim();
    let exe = if let Some(rest) = cmd.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        // Без кавычек путь может содержать пробелы — режем по «.exe».
        match cmd.to_ascii_lowercase().find(".exe") {
            Some(i) => &cmd[..i + 4],
            None    => cmd.split_whitespace().next().unwrap_or(""),
        }
    };
    let exe = exe.trim();
    (!exe.is_empty()).then(|| exe.to_string())
}

#[cfg(windows)]
mod reg {
    use std::ffi::c_void;

    pub const HKEY_CLASSES_ROOT: isize = 0x8000_0000_u32 as i32 as isize;
    pub const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x0000_0002; // REG_EXPAND_SZ раскрывается сам

    // Комментарий обычный, не `///`: к `extern`-блоку doc-комментарий не крепится.
    // RegGetValueW — с Vista, на Win7 есть. Через reg.exe нельзя: его вывод в
    // OEM-кодировке, и путь с кириллицей в имени профиля приходит испорченным.
    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(hkey: isize, sub_key: *const u16, value: *const u16, flags: u32,
                        ty: *mut u32, data: *mut c_void, cb_data: *mut u32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

    /// Строковое значение; `value = None` — значение по умолчанию ключа.
    pub fn read_sz(root: isize, sub_key: &str, value: Option<&str>) -> Option<String> {
        let key = wide(sub_key);
        let val = value.map(wide);
        let val_ptr = val.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
        let mut cb: u32 = 0;
        // SAFETY: строки с нулём на конце живут до конца функции; первый вызов
        // только узнаёт размер, второй пишет не больше `cb` байт в буфер.
        unsafe {
            if RegGetValueW(root, key.as_ptr(), val_ptr, RRF_RT_REG_SZ,
                            std::ptr::null_mut(), std::ptr::null_mut(), &mut cb) != 0 || cb == 0 {
                return None;
            }
            let mut buf = vec![0u16; (cb as usize).div_ceil(2)];
            if RegGetValueW(root, key.as_ptr(), val_ptr, RRF_RT_REG_SZ,
                            std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut cb) != 0 {
                return None;
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end]))
        }
    }
}

/// Exe браузера по умолчанию в Windows: `UserChoice` (https, затем http) →
/// ProgId → `HKCR\{ProgId}\shell\open\command`. Ключ `UserChoice` есть и на Win7.
#[cfg(windows)]
pub(crate) fn system_default_browser() -> Option<String> {
    for scheme in ["https", "http"] {
        let key = format!(
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice");
        let Some(progid) = reg::read_sz(reg::HKEY_CURRENT_USER, &key, Some("ProgId")) else { continue };
        let cmd = reg::read_sz(reg::HKEY_CLASSES_ROOT, &format!(r"{progid}\shell\open\command"), None);
        if let Some(exe) = cmd.as_deref().and_then(exe_from_command) { return Some(exe); }
    }
    None
}

#[cfg(not(windows))]
pub(crate) fn system_default_browser() -> Option<String> { None }

/// Что запускать для снимка: файл на месте и браузер умеет снимать. У Opera —
/// движок из папки версии; нет движка — «не найден» (лаунчер снимок не
/// сделает), движок новее 95 — «не умеет снимать».
/// `Err` — окончание фразы для причины в `choose`.
pub(crate) fn runnable(path: &str) -> Result<String, String> {
    const MISSING: &str = "не найден";
    let p = Path::new(path);
    if !p.is_file() { return Err(MISSING.into()); }
    match engine_of(path) {
        Engine::Chromium => Ok(path.to_string()),
        Engine::Opera    => match find_opera_engine(p) {
            None => Err(MISSING.into()),
            Some(e) if opera_engine_can_shoot(&e) => Ok(e.to_string_lossy().into_owned()),
            Some(_) => Err(format!("не умеет снимать (Opera новее {OPERA_LAST_SHOOTING})")),
        },
        Engine::None     => Err("не умеет снимать".into()),
    }
}

/// Снять браузер со всеми дочерними процессами. `Child::kill` убивает только
/// родителя, а рендереры и GPU-процесс headless-браузера остаются висеть и
/// держат профиль. (Наработка url-album-3.)
pub(crate) fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output();
    }
    #[cfg(not(windows))]
    let _ = pid;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn exe_from_shell_command() {
        assert_eq!(exe_from_command(r#""C:\Users\Коля\AppData\Local\Programs\Opera\launcher.exe" -noautoupdate -- "%1""#).as_deref(),
                   Some(r"C:\Users\Коля\AppData\Local\Programs\Opera\launcher.exe"));
        assert_eq!(exe_from_command(r"C:\Program Files\Internet Explorer\iexplore.exe %1").as_deref(),
                   Some(r"C:\Program Files\Internet Explorer\iexplore.exe"), "без кавычек, с пробелами в пути");
        assert_eq!(exe_from_command(r#"  "C:\x\chrome.EXE" --single-argument %1"#).as_deref(), Some(r"C:\x\chrome.EXE"));
        assert_eq!(exe_from_command(""), None);
        assert_eq!(exe_from_command(r#""""#), None);
    }

    #[test]
    fn runnable_checks_file_and_engine() {
        let d = tmp_dir("runnable");
        let chrome = d.join("chrome.exe");
        touch(&chrome);
        let ff = d.join("firefox.exe");
        touch(&ff);
        let op = d.join("Opera");
        touch(&op.join("launcher.exe"));
        touch(&op.join("95.0.4635.90").join("opera.exe"));
        let s = |p: &Path| p.to_string_lossy().into_owned();

        assert_eq!(runnable(&s(&chrome)), Ok(s(&chrome)));
        assert_eq!(runnable(&s(&ff)), Err("не умеет снимать".into()), "есть, но снимать не умеет");
        assert_eq!(runnable(&s(&d.join("msedge.exe"))), Err("не найден".into()), "файла нет");
        assert_eq!(runnable(&s(&op.join("launcher.exe"))), Ok(s(&op.join("95.0.4635.90").join("opera.exe"))));

        let new = d.join("Opera New");
        touch(&new.join("launcher.exe"));
        touch(&new.join("137.0.6036.63").join("opera.exe"));
        assert!(runnable(&s(&new.join("launcher.exe"))).is_err_and(|e| e.starts_with("не умеет снимать")),
                "Opera 137 есть, но не снимает");
        let bare = d.join("Opera Bare");
        touch(&bare.join("launcher.exe"));
        assert_eq!(runnable(&s(&bare.join("launcher.exe"))), Err("не найден".into()), "лаунчер без движка");
    }

    #[test]
    fn opera_version_boundary() {
        let e = |ver: &str| PathBuf::from(format!(r"C:\Opera\{ver}\opera.exe"));
        assert!(opera_engine_can_shoot(&e("95.0.4635.90")));
        assert!(opera_engine_can_shoot(&e("94.0.4606.76")));
        assert!(!opera_engine_can_shoot(&e("96.0.1.1")));
        assert!(!opera_engine_can_shoot(&e("137.0.6036.63")));
        assert!(!opera_engine_can_shoot(&PathBuf::from(r"C:\Opera\opera.exe")), "папка не версия");
    }

    #[test]
    fn engine_by_exe_name() {
        for p in [r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                  r"C:\Users\u\AppData\Local\Google\Chrome\Application\CHROME.EXE",
                  r"C:\Users\u\AppData\Local\Yandex\YandexBrowser\Application\browser.exe",
                  // Путь из browsers.json мог быть вписан руками с прямыми слэшами.
                  "C:/Users/u/AppData/Local/Yandex/YandexBrowser/Application/browser.exe"] {
            assert_eq!(engine_of(p), Engine::Chromium, "{p}");
        }
        for p in [r"C:\Users\u\AppData\Local\Programs\Opera\launcher.exe",
                  r"C:\Users\u\AppData\Local\Programs\Opera\95.0.4635.90\opera.exe",
                  r"C:\Users\u\AppData\Local\Programs\Opera GX\Launcher.exe"] {
            assert_eq!(engine_of(p), Engine::Opera, "{p}");
        }
        for p in [r"C:\Program Files\Mozilla Firefox\firefox.exe",
                  r"C:\Program Files\Internet Explorer\iexplore.exe",
                  r"D:\Tor Browser\Browser\firefox.exe",
                  // `--screenshot` не делают ни с одним флагом (проверено 19.10.2026).
                  r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
                  r"C:\Users\u\AppData\Local\Vivaldi\Application\vivaldi.exe",
                  r"C:\Some\launcher.exe",        // чужой лаунчер — не Opera
                  r"C:\Some\browser.exe",         // browser.exe не из Яндекса
                  "default", ""] {
            assert_eq!(engine_of(p), Engine::None, "{p}");
        }
    }

    #[test]
    fn headless_flags_one_attempt() {
        assert_eq!(headless_flags(Engine::Opera), &["--headless"], "Opera 95 снимает только старым headless");
        assert_eq!(headless_flags(Engine::Chromium), &["--headless=new"]);
        assert!(headless_flags(Engine::None).is_empty());
    }

    #[test]
    fn choose_new_opera_is_skipped() {
        // Настоящий `runnable` на временных папках: решает версия движка.
        let d = tmp_dir("opera_ver");
        let chrome = d.join("chrome.exe");
        touch(&chrome);
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let opera = |ver: &str| {
            let dir = d.join(format!("Opera_{ver}"));
            touch(&dir.join("launcher.exe"));
            touch(&dir.join(ver).join("opera.exe"));
            (s(&dir.join("launcher.exe")), s(&dir.join(ver).join("opera.exe")))
        };
        let pick = |default: &str| {
            let c = cfg(default, &[default, &s(&chrome)]);
            let inp = Inputs { opener: None, cfg: Some(&c), builtin: &[] };
            choose(&inp, &|| None, &runnable).unwrap()
        };

        let (l95, e95) = opera("95.0.4635.90");
        let got = pick(&l95);
        assert_eq!(got.exe, e95, "Opera 95 снимает сама");
        assert_eq!(got.fallback, None);

        for ver in ["96.0.1.1", "137.0.6036.63"] {
            let (l, _) = opera(ver);
            let got = pick(&l);
            assert_eq!(got.exe, s(&chrome), "Opera {ver}: снимает следующий по порядку");
            assert!(got.fallback.as_deref().is_some_and(|r| r.contains("Opera") && r.contains("не умеет")),
                    "Opera {ver}: {:?}", got.fallback);
        }
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name(r"C:\x\msedge.exe"), "Edge");
        assert_eq!(display_name(r"C:\x\chrome.exe"), "Chrome");
        assert_eq!(display_name(r"C:\x\Opera\launcher.exe"), "Opera");
        assert_eq!(display_name(r"C:\x\Opera GX\launcher.exe"), "Opera GX");
        assert_eq!(display_name(r"C:\x\Yandex\YandexBrowser\Application\browser.exe"), "Яндекс");
        assert_eq!(display_name("C:/x/Yandex/YandexBrowser/Application/browser.exe"), "Яндекс");
        assert_eq!(display_name(r"C:\x\firefox.exe"), "Firefox");
        assert_eq!(display_name(r"C:\x\iexplore.exe"), "Internet Explorer");
        assert_eq!(display_name(r"C:\x\Thorium.exe"), "Thorium", "незнакомый — имя файла");
    }

    /// Временная папка теста; удаляется в `Drop` — и когда тест упал на
    /// `assert` (раскрутка паники тоже зовёт `Drop`), иначе в %TEMP% копились
    /// `ua_shot_test_*` от каждого красного прогона.
    struct TmpDir(PathBuf);

    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    impl std::ops::Deref for TmpDir {
        type Target = Path;
        fn deref(&self) -> &Path { &self.0 }
    }

    /// Имя с pid и номером, чтобы параллельные прогоны не делили папку (на
    /// этом уже спотыкался импорт Firefox).
    fn tmp_dir(tag: &str) -> TmpDir {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "ua_shot_test_{}_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TmpDir(d)
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"").unwrap();
    }

    #[test]
    fn opera_engine_is_highest_version() {
        let d = tmp_dir("opera");
        let launcher = d.join("launcher.exe");
        touch(&launcher);
        touch(&d.join("opera.exe"));
        touch(&d.join("94.0.4606.76").join("opera.exe"));
        touch(&d.join("95.0.4635.90").join("opera.exe"));
        touch(&d.join("95.0.4635.100").join("opera.exe")); // 100 > 90 числом, не строкой
        std::fs::create_dir_all(d.join("96.0.1.1")).unwrap(); // версия без opera.exe
        touch(&d.join("assistant").join("opera.exe"));     // не версия
        touch(&d.join("1.2.x").join("opera.exe"));         // не версия

        let want = d.join("95.0.4635.100").join("opera.exe");
        assert_eq!(find_opera_engine(&launcher), Some(want.clone()), "от лаунчера");
        assert_eq!(find_opera_engine(&d.join("opera.exe")), Some(want.clone()), "от корневого opera.exe");
        let ver = d.join("95.0.4635.90").join("opera.exe");
        assert_eq!(find_opera_engine(&ver), Some(ver.clone()), "уже движок — он сам");

        let empty = tmp_dir("opera_empty");
        touch(&empty.join("launcher.exe"));
        assert_eq!(find_opera_engine(&empty.join("launcher.exe")), None, "без папок версий — не лаунчер");
    }

    #[test]
    fn browsers_json_parsing() {
        let raw = "\u{feff}{\"browsers\":[{\"name\":\"Браузер по умолчанию\",\"path\":\"default\"},\
                   {\"name\":\"Opera\",\"path\":\"C:\\\\O\\\\launcher.exe\"}],\"default\":\"C:\\\\O\\\\launcher.exe\"}";
        let cfg = parse_browsers_json(raw).expect("BOM не мешает");
        assert_eq!(cfg.default, r"C:\O\launcher.exe");
        assert_eq!(cfg.list, vec![
            ("Браузер по умолчанию".to_string(), "default".to_string()),
            ("Opera".to_string(), r"C:\O\launcher.exe".to_string()),
        ]);

        let no_default = parse_browsers_json(r#"{"browsers":[{"name":"A","path":"C:\\a.exe"}]}"#).unwrap();
        assert_eq!(no_default.default, "default", "нет ключа — как в app.js");

        let junk = parse_browsers_json(r#"{"browsers":[{"name":"A"},{"path":""},7,{"name":"B","path":"C:\\b.exe"}],"default":""}"#).unwrap();
        assert_eq!(junk.list, vec![("B".to_string(), r"C:\b.exe".to_string())], "записи без пути пропускаются");
        assert_eq!(junk.default, "default", "пустой default — как не задан");

        assert_eq!(parse_browsers_json(""), None, "файла нет");
        assert_eq!(parse_browsers_json("{не json"), None, "испорчен");
    }

    #[test]
    fn builtin_order() {
        let c = builtin_candidates(r"C:\L", r"C:\PF", r"C:\PF86");
        let first = |needle: &str| c.iter().position(|p| p.to_ascii_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("нет {needle} в {c:?}"));
        let order = ["msedge.exe", "chrome.exe", r"\opera\", r"\yandex\"];
        let pos: Vec<usize> = order.iter().map(|n| first(n)).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "порядок Edge → Chrome → Opera → Яндекс: {c:?}");
        assert!(c.iter().any(|p| p == r"C:\L\Google\Chrome\Application\chrome.exe"), "Chrome только для пользователя");
        assert!(c.iter().any(|p| p == r"C:\L\Programs\Opera\launcher.exe"), "Opera для пользователя (Win7)");
        assert!(c.iter().all(|p| engine_of(p) != Engine::None), "все кандидаты умеют снимать: {c:?}");
        assert!(!c.iter().any(|p| { let p = p.to_ascii_lowercase(); p.contains("brave") || p.contains("vivaldi") }),
                "Brave и Vivaldi снимать не умеют — во встроенном порядке им нечего делать: {c:?}");
    }

    // ── choose ──────────────────────────────────────────────────────────────

    const EDGE:    &str = r"C:\PF86\Microsoft\Edge\Application\msedge.exe";
    const CHROME:  &str = r"C:\PF\Google\Chrome\Application\chrome.exe";
    const OPERA:   &str = r"C:\L\Programs\Opera\launcher.exe";
    const OPERA_E: &str = r"C:\L\Programs\Opera\95.0.4635.90\opera.exe";
    const VIVALDI: &str = r"C:\L\Vivaldi\Application\vivaldi.exe";
    const FIREFOX: &str = r"C:\PF\Mozilla Firefox\firefox.exe";
    const TOR:     &str = r"D:\Tor Browser\Browser\firefox.exe";

    /// Мир теста: какие файлы есть и чем их запускать.
    fn world(present: &[&str]) -> impl Fn(&str) -> Result<String, String> {
        let mut m: HashMap<String, String> = HashMap::new();
        for p in present {
            if engine_of(p) == Engine::None { continue; } // есть, но снимать не умеет
            let run = if *p == OPERA { OPERA_E } else { p };
            m.insert(p.to_string(), run.to_string());
        }
        move |p: &str| m.get(p).cloned().ok_or_else(|| "не найден".to_string())
    }

    fn cfg(default: &str, list: &[&str]) -> BrowsersCfg {
        BrowsersCfg {
            default: default.to_string(),
            list: list.iter().map(|p| (display_name(p), p.to_string())).collect(),
        }
    }

    #[test]
    fn choose_folder_opener_first() {
        let c = cfg(CHROME, &["default", CHROME, OPERA]);
        let builtin = vec![EDGE.to_string()];
        let inp = Inputs { opener: Some(OPERA), cfg: Some(&c), builtin: &builtin };
        let got = choose(&inp, &|| Some(EDGE.to_string()), &world(&[EDGE, CHROME, OPERA])).unwrap();
        assert_eq!(got.exe, OPERA_E, "Opera запускается движком, не лаунчером");
        assert_eq!(got.name, "Opera");
        assert_eq!(got.fallback, None);
    }

    #[test]
    fn choose_folder_opener_cannot_shoot_goes_to_default() {
        let c = cfg(CHROME, &["default", CHROME]);
        let inp = Inputs { opener: Some(TOR), cfg: Some(&c), builtin: &[] };
        let got = choose(&inp, &|| None, &world(&[TOR, CHROME])).unwrap();
        assert_eq!(got.exe, CHROME, "дальше по общему правилу — браузер по умолчанию программы");
        assert!(got.fallback.as_deref().is_some_and(|r| r.contains("Firefox")), "причина называет браузер папки: {:?}", got.fallback);
    }

    #[test]
    fn choose_program_default_is_used() {
        // Случай с Win7: Opera выбрана по умолчанию в «Браузерах».
        let c = cfg(OPERA, &["default", OPERA]);
        let builtin = vec![EDGE.to_string(), CHROME.to_string()];
        let inp = Inputs { opener: None, cfg: Some(&c), builtin: &builtin };
        let got = choose(&inp, &|| Some(EDGE.to_string()), &world(&[EDGE, OPERA])).unwrap();
        assert_eq!(got.exe, OPERA_E, "выбор программы важнее браузера Windows и встроенного порядка");
        assert_eq!(got.fallback, None);
    }

    #[test]
    fn choose_windows_default_when_default_entry() {
        let c = cfg("default", &["default", EDGE, CHROME]);
        let inp = Inputs { opener: None, cfg: Some(&c), builtin: &[] };
        let got = choose(&inp, &|| Some(CHROME.to_string()), &world(&[EDGE, CHROME])).unwrap();
        assert_eq!(got.exe, CHROME);
        assert_eq!(got.fallback, None, "браузер Windows — это и есть выбор, не запасной");

        // Vivaldi в Windows: снимать не умеет — запасной из списка, с причиной.
        let got = choose(&inp, &|| Some(VIVALDI.to_string()), &world(&[VIVALDI, EDGE, CHROME])).unwrap();
        assert_eq!(got.exe, EDGE);
        assert!(got.fallback.as_deref().is_some_and(|r| r.contains("Vivaldi") && r.contains("не умеет")),
                "{:?}", got.fallback);
    }

    #[test]
    fn choose_fallback_list_order_then_builtin() {
        // По умолчанию Firefox: снимать не умеет → первый умеющий из списка.
        let c = cfg(FIREFOX, &["default", FIREFOX, VIVALDI, CHROME]);
        let builtin = vec![EDGE.to_string()];
        let inp = Inputs { opener: None, cfg: Some(&c), builtin: &builtin };
        let got = choose(&inp, &|| Some(FIREFOX.to_string()), &world(&[FIREFOX, VIVALDI, CHROME, EDGE])).unwrap();
        assert_eq!(got.exe, CHROME, "порядок списка, а не встроенный; Vivaldi пропущен");
        assert!(got.fallback.as_deref().is_some_and(|r| r.contains("Firefox")), "{:?}", got.fallback);

        // В списке умеющих нет — встроенный порядок.
        let c = cfg(FIREFOX, &["default", FIREFOX]);
        let builtin = vec![EDGE.to_string(), CHROME.to_string()];
        let inp = Inputs { opener: None, cfg: Some(&c), builtin: &builtin };
        let got = choose(&inp, &|| Some(FIREFOX.to_string()), &world(&[FIREFOX, CHROME])).unwrap();
        assert_eq!(got.exe, CHROME, "Edge нет на диске — следующий");
        assert!(got.fallback.is_some());
    }

    #[test]
    fn choose_missing_default_file_is_fallback() {
        // Портативный браузер уехал вместе с флешкой.
        let c = cfg(r"E:\Portable\chrome.exe", &[r"E:\Portable\chrome.exe", OPERA]);
        let inp = Inputs { opener: None, cfg: Some(&c), builtin: &[] };
        let got = choose(&inp, &|| None, &world(&[OPERA])).unwrap();
        assert_eq!(got.exe, OPERA_E);
        assert!(got.fallback.as_deref().is_some_and(|r| r.contains("Chrome")), "{:?}", got.fallback);
    }

    #[test]
    fn choose_without_browsers_json() {
        let builtin = vec![EDGE.to_string(), CHROME.to_string(), OPERA.to_string()];
        let inp = Inputs { opener: None, cfg: None, builtin: &builtin };
        // Нет файла — как "default" в app.js: браузер Windows.
        let got = choose(&inp, &|| Some(OPERA.to_string()), &world(&[CHROME, OPERA])).unwrap();
        assert_eq!(got.exe, OPERA_E);
        assert_eq!(got.fallback, None);
        // Браузер Windows не определён — встроенный порядок, с пометкой.
        let got = choose(&inp, &|| None, &world(&[CHROME, OPERA])).unwrap();
        assert_eq!(got.exe, CHROME);
        assert!(got.fallback.is_some());
    }

    #[test]
    fn choose_nothing_suitable() {
        let c = cfg(FIREFOX, &["default", FIREFOX]);
        let builtin = vec![EDGE.to_string()];
        let inp = Inputs { opener: Some(TOR), cfg: Some(&c), builtin: &builtin };
        assert_eq!(choose(&inp, &|| Some(FIREFOX.to_string()), &world(&[FIREFOX, TOR])), None);
    }
}
