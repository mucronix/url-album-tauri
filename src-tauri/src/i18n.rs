// ── Словари интерфейса ───────────────────────────────────────────────────────
//
// Словари вшиты в exe (`include_str!`); базовый — русский, в нём есть все
// ключи. Необязательная папка `lang\` рядом с exe подменяет вшитый язык
// (`lang\ru.json`) или добавляет новый (`lang\de.json`). Язык выбирается один
// раз при запуске: смена языка — после перезапуска.
//
// Формат: плоский объект «ключ → строка». Подстановки — `{имя}`. Фраза с
// числом — объект категорий `Intl.PluralRules` (`one`, `few`, `many`, `other`),
// число в ней — `{n}`; склоняет только JS, Rust числа не склоняет.
//
// Недостающий ключ берётся по цепочке: `lang\<язык>.json` → вшитый `<язык>` →
// вшитый `en` → вшитый `ru`. Испорченный ключ (не того вида, чужие `{…}`)
// считается недостающим. Битый файл (не разобран JSON) не читается целиком;
// JS показывает об этом окно (`broken`).
//
// Журнал не переводится: `t_log!` всегда берёт вшитый русский словарь.

use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::Path;
use std::sync::OnceLock;
use serde_json::{Map, Value};

/// Базовый язык: в его словаре есть все ключи.
pub const BASE_LANG: &str = "ru";

/// Вшитые словари. Новый язык — строкой сюда и файлом в `lang/`.
const EMBEDDED: &[(&str, &str)] = &[
    ("ru", include_str!("../lang/ru.json")),
];

/// Категории `Intl.PluralRules`, допустимые в объекте фразы с числом.
const PLURAL_CATEGORIES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// Выбранный язык и итоговый словарь.
pub struct Dict {
    pub lang:    String,
    /// Слитый словарь: каждый ключ базового есть и здесь.
    pub strings: Map<String, Value>,
    /// Файлы `lang\`, которые не разобрались, — для окна в JS.
    pub broken:  Vec<String>,
    /// Строки для журнала: пишутся в setup, после шапки запуска.
    pub notes:   Vec<String>,
}

/// Текст фразы для интерфейса — на выбранном языке.
/// `t!("ключ")`, `t!("ключ", имя = значение, …)`, `t!("ключ", имя)` (= `имя = имя`).
#[macro_export]
macro_rules! t {
    ($key:literal $(, $name:ident $(= $val:expr)?)* $(,)?) => {
        $crate::i18n::tr($key, &[$((stringify!($name), $crate::i18n_arg!($name $(= $val)?))),*])
    };
}

/// То же для журнала — всегда по вшитому русскому словарю.
#[macro_export]
macro_rules! t_log {
    ($key:literal $(, $name:ident $(= $val:expr)?)* $(,)?) => {
        $crate::i18n::tr_log($key, &[$((stringify!($name), $crate::i18n_arg!($name $(= $val)?))),*])
    };
}

/// Фраза для интерфейса и журнала сразу — `Msg { ui, log }`.
#[macro_export]
macro_rules! t_msg {
    ($($all:tt)*) => {
        $crate::i18n::Msg { ui: $crate::t!($($all)*), log: $crate::t_log!($($all)*) }
    };
}

#[macro_export]
macro_rules! i18n_arg {
    ($name:ident = $val:expr) => { &$val as &dyn std::fmt::Display };
    ($name:ident)             => { &$name as &dyn std::fmt::Display };
}

/// Фраза, которая уходит и в интерфейс, и в журнал (причина отказа открытия,
/// снимка, запасного браузера): `ui` — на языке интерфейса, `log` — по
/// вшитому русскому. `t_msg!` — как `t!`, аргументы вычисляются дважды.
#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub ui:  String,
    pub log: String,
}

static CURRENT: OnceLock<Dict> = OnceLock::new();
static BASE: OnceLock<Map<String, Value>> = OnceLock::new();

/// Разобрать словарь: объект JSON, BOM срезается (файл, сохранённый Блокнотом).
fn parse(text: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_)  => Err("не объект JSON".to_string()),   // i18n: не переводится — журнал
        Err(e) => Err(e.to_string()),
    }
}

fn embedded(lang: &str) -> Option<&'static str> {
    EMBEDDED.iter().find(|(l, _)| *l == lang).map(|(_, text)| *text)
}

/// Вшитый русский словарь. Что он разбирается — сторожит тест.
pub fn base() -> &'static Map<String, Value> {
    BASE.get_or_init(|| parse(embedded(BASE_LANG).unwrap_or("{}")).unwrap_or_default())
}

/// Код языка из настройки, системы или имени файла: основной подтег,
/// строчными («en-US» → «en»). Не буквы — None (в том числе «..\x»).
pub fn normalize_code(s: &str) -> Option<String> {
    let primary = s.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    let ok = (2..=3).contains(&primary.len()) && primary.bytes().all(|b| b.is_ascii_lowercase());
    ok.then_some(primary)
}

/// Имена подстановок `{имя}` в строке.
fn placeholders(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else { break };
        let name = &rest[..end];
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            out.insert(name.to_string());
            rest = &rest[end + 1..];
        }
    }
    out
}

fn plural_placeholders(m: &Map<String, Value>) -> Option<BTreeSet<String>> {
    let mut all = BTreeSet::new();
    for (cat, v) in m {
        if !PLURAL_CATEGORIES.contains(&cat.as_str()) { return None; }
        all.extend(placeholders(v.as_str()?));
    }
    Some(all)
}

/// Годится ли перевод `v` на место базового `base_v`: тот же вид (строка или
/// фраза с числом), в строке — те же подстановки, в фразе с числом — не
/// больше, чем в базовой (форма «one» может обойтись без `{n}`), и есть `other`.
fn value_ok(base_v: &Value, v: &Value) -> bool {
    match (base_v, v) {
        (Value::String(b), Value::String(s)) => placeholders(b) == placeholders(s),
        (Value::Object(b), Value::Object(m)) => {
            let (Some(bp), Some(mp)) = (plural_placeholders(b), plural_placeholders(m)) else { return false };
            m.contains_key("other") && mp.is_subset(&bp)
        }
        _ => false,
    }
}

/// Наложить словарь `from` на `into`: только ключи базового и только годные
/// значения. Отвергнутые ключи — в `rejected`.
fn overlay(into: &mut Map<String, Value>, from: &Map<String, Value>,
           base: &Map<String, Value>, rejected: &mut Vec<String>) {
    for (k, v) in from {
        if k == "_meta" { continue; }
        match base.get(k) {
            Some(bv) if value_ok(bv, v) => { into.insert(k.clone(), v.clone()); }
            _ => rejected.push(k.clone()),
        }
    }
}

/// Порядок кандидатов: настройка, язык системы, английский, базовый.
fn candidates(settings: Option<&str>, system: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in [settings.and_then(normalize_code), system.and_then(normalize_code),
              Some("en".to_string()), Some(BASE_LANG.to_string())].into_iter().flatten() {
        if !out.contains(&c) { out.push(c); }
    }
    out
}

/// Выбрать язык и собрать словарь. `base_text` и `embedded` — параметры ради
/// тестов; в программе — вшитые.
fn load_with(base_text: &str, embedded: &dyn Fn(&str) -> Option<&'static str>,
             lang_dir: Option<&Path>, settings: Option<&str>, system: Option<&str>) -> Dict {
    let base = parse(base_text).unwrap_or_default();
    let mut broken = Vec::new();
    let mut notes  = Vec::new();

    // Файл lang\<код>.json: нет — None; не разобрался — в broken, тоже None.
    let mut read_file = |code: &str| -> Option<Map<String, Value>> {
        let name = format!("{code}.json");
        let text = std::fs::read_to_string(lang_dir?.join(&name)).ok()?;
        match parse(&text) {
            Ok(m)  => Some(m),
            Err(e) => {
                notes.push(format!("язык: lang\\{name} не разобран ({e}) — не используется"));   // i18n: не переводится — журнал
                broken.push(name);
                None
            }
        }
    };

    let mut lang = BASE_LANG.to_string();
    let mut file = None;
    for code in candidates(settings, system) {
        let f = read_file(&code);
        if f.is_some() || embedded(&code).is_some() {
            lang = code;
            file = f;
            break;
        }
    }

    let mut strings = base.clone();
    let layers = [
        ("вшитый en".to_string(),      embedded("en").filter(|_| lang != "en" && lang != BASE_LANG).and_then(|t| parse(t).ok())),   // i18n: не переводится — журнал
        (format!("вшитый {lang}"),  embedded(&lang).filter(|_| lang != BASE_LANG).and_then(|t| parse(t).ok())),   // i18n: не переводится — журнал
        (format!("lang\\{lang}.json"), file),
    ];
    for (name, layer) in layers {
        let Some(layer) = layer else { continue };
        let mut rejected = Vec::new();
        overlay(&mut strings, &layer, &base, &mut rejected);
        if !rejected.is_empty() {
            notes.push(format!("язык: {name} — ключи пропущены (неизвестные или неверные): {}",   // i18n: не переводится — журнал
                               rejected.join(", ")));
        }
    }
    Dict { lang, strings, broken, notes }
}

/// Язык интерфейса Windows: «ru-RU», «en-US». None — не узнать.
#[cfg(windows)]
fn system_language() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
        fn LCIDToLocaleName(locale: u32, name: *mut u16, cch: i32, flags: u32) -> i32;
    }
    let mut buf = [0u16; 85];   // LOCALE_NAME_MAX_LENGTH
    let n = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, buf.as_mut_ptr(), buf.len() as i32, 0) };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
}
#[cfg(not(windows))]
fn system_language() -> Option<String> { None }

/// Выбор языка при запуске: `language` из settings.json, язык системы, `lang\`
/// рядом с exe. Зовётся из main() до Builder: меню трея строится в setup.
/// Выбранный язык в settings.json не пишется: появится `en.json` — английская
/// Windows получит его сама, а не застрянет на записанном когда-то `ru`.
pub fn init() {
    let settings = serde_json::from_str::<Value>(&crate::read_settings_raw()).ok()
        .and_then(|v| v.get("language").and_then(|x| x.as_str()).map(str::to_string));
    let system = system_language();
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("lang")));
    let mut d = load_with(embedded(BASE_LANG).unwrap_or("{}"), &embedded, dir.as_deref(),
                          settings.as_deref(), system.as_deref());
    d.notes.insert(0, format!("язык: {} (настройка: {}, система: {})", d.lang,   // i18n: не переводится — журнал
                              settings.as_deref().unwrap_or("нет"), system.as_deref().unwrap_or("не определён")));   // i18n: не переводится — журнал
    let _ = CURRENT.set(d);
}

/// Текущий словарь; до init() (и в тестах) — базовый.
pub fn current() -> &'static Dict {
    CURRENT.get_or_init(|| Dict {
        lang: BASE_LANG.to_string(), strings: base().clone(), broken: Vec::new(), notes: Vec::new(),
    })
}

/// Подставить аргументы в строку словаря. Нет ключа или это фраза с числом
/// (её склоняет JS) — заметная метка `⟦ключ⟧`, не пустота.
fn render(map: &Map<String, Value>, key: &str, args: &[(&str, &dyn Display)]) -> String {
    let Some(text) = map.get(key).and_then(Value::as_str) else { return format!("⟦{key}⟧") };
    let mut out = text.to_string();
    for (name, val) in args {
        out = out.replace(&format!("{{{name}}}"), &val.to_string());
    }
    out
}

pub fn tr(key: &str, args: &[(&str, &dyn Display)]) -> String {
    render(&current().strings, key, args)
}

pub fn tr_log(key: &str, args: &[(&str, &dyn Display)]) -> String {
    render(base(), key, args)
}

/// Скрипт для WebView2: словарь — в `window.__UA_I18N__` до загрузки app.js,
/// чтобы `t()` работал и в константах верхнего уровня, и в разметке до
/// первой отрисовки.
pub fn init_script() -> String {
    let d = current();
    let data = serde_json::json!({ "lang": d.lang, "strings": d.strings, "broken": d.broken });
    format!("window.__UA_I18N__ = {data};")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_embedded(_: &str) -> Option<&'static str> { None }

    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("ua_i18n_test_{tag}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
        fn write(&self, name: &str, text: &str) { std::fs::write(self.0.join(name), text).unwrap(); }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    const BASE_T: &str = r#"{
        "_meta": { "lang": "ru" },
        "a": "Файл {path}",
        "b": "Готово",
        "n": { "one": "{n} ссылка", "few": "{n} ссылки", "many": "{n} ссылок", "other": "{n} ссылки" }
    }"#;

    fn s<'a>(d: &'a Dict, k: &str) -> &'a Value { d.strings.get(k).unwrap() }

    /// Вшитый ru.json разбирается, и у каждой фразы с числом есть все категории
    /// русского: иначе цепочка у вшитых словарей дошла бы до ⟦ключ⟧.
    #[test]
    fn embedded_ru_parses_and_plurals_complete() {
        let m = parse(embedded(BASE_LANG).unwrap()).expect("вшитый ru.json не разобрался");
        for (k, v) in &m {
            if k == "_meta" { continue; }
            match v {
                Value::String(_) => {}
                Value::Object(o) => for cat in ["one", "few", "many", "other"] {
                    assert!(o.get(cat).is_some_and(Value::is_string), "{k}: нет категории {cat}");
                },
                _ => panic!("{k}: не строка и не фраза с числом"),
            }
        }
    }

    #[test]
    fn normalize_code_takes_primary_subtag() {
        assert_eq!(normalize_code("en-US").as_deref(), Some("en"));
        assert_eq!(normalize_code("UK").as_deref(), Some("uk"));
        assert_eq!(normalize_code("pt_BR").as_deref(), Some("pt"));
        assert_eq!(normalize_code(""), None);
        assert_eq!(normalize_code("..\\x"), None);
        assert_eq!(normalize_code("e1"), None);
    }

    #[test]
    fn language_order_settings_system_en_ru() {
        let d = TmpDir::new("order");
        d.write("de.json", r#"{"b": "Fertig"}"#);
        d.write("fr.json", r#"{"b": "Prêt"}"#);
        let pick = |set: Option<&str>, sys: Option<&str>|
            load_with(BASE_T, &no_embedded, Some(&d.0), set, sys).lang;
        assert_eq!(pick(Some("de"), Some("fr-FR")), "de", "настройка — первой");
        assert_eq!(pick(Some("xx"), Some("fr-FR")), "fr", "настройки нет в словарях — язык системы");
        assert_eq!(pick(None, Some("en-US")), "ru", "английского нет — базовый");
        let en_only = |l: &str| (l == "en").then_some(r#"{"b": "Done"}"#);
        assert_eq!(load_with(BASE_T, &en_only, Some(&d.0), None, Some("ja-JP")).lang, "en",
                   "ни настройки, ни системы — английский");
    }

    #[test]
    fn partial_file_falls_back_to_base() {
        let d = TmpDir::new("partial");
        d.write("ru.json", r#"{"b": "Сделано"}"#);
        let r = load_with(BASE_T, &no_embedded, Some(&d.0), None, None);
        assert_eq!(r.lang, "ru");
        assert_eq!(s(&r, "b"), "Сделано", "lang\\ru.json подменяет вшитое");
        assert_eq!(s(&r, "a"), "Файл {path}", "недостающий ключ — из базового");
        assert!(r.broken.is_empty() && r.notes.is_empty());
    }

    #[test]
    fn broken_file_is_ignored_and_reported() {
        let d = TmpDir::new("broken");
        d.write("ru.json", "{ \"b\": ");
        let r = load_with(BASE_T, &no_embedded, Some(&d.0), None, None);
        assert_eq!(s(&r, "b"), "Готово");
        assert_eq!(r.broken, vec!["ru.json".to_string()]);
        assert!(r.notes.iter().any(|n| n.contains("lang\\ru.json")), "{:?}", r.notes);
    }

    #[test]
    fn broken_new_language_falls_through() {
        let d = TmpDir::new("broken_new");
        d.write("de.json", "[1, 2");
        let r = load_with(BASE_T, &no_embedded, Some(&d.0), Some("de"), None);
        assert_eq!(r.lang, "ru", "битый de.json — языка нет, дальше по порядку");
        assert_eq!(r.broken, vec!["de.json".to_string()]);
    }

    #[test]
    fn bad_values_are_rejected() {
        let d = TmpDir::new("bad");
        d.write("de.json", r#"{
            "a": "Datei {pfad}",
            "b": { "one": "x", "other": "y" },
            "n": { "one": "ein Link", "other": "{n} Links {x}" },
            "zzz": "unbekannt",
            "_meta": { "lang": "de" }
        }"#);
        let r = load_with(BASE_T, &no_embedded, Some(&d.0), Some("de"), None);
        assert_eq!(r.lang, "de");
        assert_eq!(s(&r, "a"), "Файл {path}", "чужая подстановка — ключ из базового");
        assert_eq!(s(&r, "b"), "Готово", "фраза с числом на месте строки — из базового");
        assert!(s(&r, "n").get("many").is_some(), "чужая подстановка в фразе с числом — из базового");
        assert!(!r.strings.contains_key("zzz"), "неизвестный ключ не добавляется");
        let note = r.notes.join("\n");
        for k in ["a", "b", "n", "zzz"] { assert!(note.contains(k), "{k} не в журнале: {note}"); }
        assert!(!note.contains("_meta"));
    }

    #[test]
    fn plural_may_drop_n_in_one() {
        let d = TmpDir::new("plural");
        d.write("de.json", r#"{ "n": { "one": "ein Link", "other": "{n} Links" } }"#);
        let r = load_with(BASE_T, &no_embedded, Some(&d.0), Some("de"), None);
        assert_eq!(s(&r, "n").get("one").unwrap(), "ein Link");
        let d2 = TmpDir::new("plural_no_other");
        d2.write("de.json", r#"{ "n": { "one": "ein Link" } }"#);
        let r2 = load_with(BASE_T, &no_embedded, Some(&d2.0), Some("de"), None);
        assert!(s(&r2, "n").get("many").is_some(), "без other — из базового");
    }

    #[test]
    fn render_substitutes_and_marks_missing() {
        let m = parse(BASE_T).unwrap();
        let path = "C:\\x";
        assert_eq!(render(&m, "a", &[("path", &path)]), "Файл C:\\x");
        assert_eq!(render(&m, "нет", &[]), "⟦нет⟧");
        assert_eq!(render(&m, "n", &[]), "⟦n⟧", "фразу с числом склоняет JS");
    }

    #[test]
    fn macros_pass_named_and_shorthand_args() {
        // Сторож на будущее: ключа нет в словаре — проверяется только, что
        // макросы собираются с обеими формами аргументов.
        let who = "x";
        assert_eq!(t!("нет.такого", who, n = 1 + 1), "⟦нет.такого⟧");
        assert_eq!(t_log!("нет.такого"), "⟦нет.такого⟧");
    }
}
