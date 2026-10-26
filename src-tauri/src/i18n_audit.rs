// ── Сверка исходников со словарём (только тесты) ────────────────────────────
//
// Разметка, app.js и Rust читаются как текст и сверяются с вшитым ru.json:
// каждый ключ есть в словаре, текст разметки совпадает со словарём, вызовы
// t() не затенены. app.js разбирается небольшим лексером: строки и
// комментарии не должны давать ложных совпадений.

use std::path::{Path, PathBuf};
use serde_json::{Map, Value};

fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf() }
fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}
fn dict() -> Map<String, Value> {
    serde_json::from_str(&read("lang/ru.json")).expect("ru.json не разобрался")
}
fn has_cyrillic(s: &str) -> bool { s.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c)) }
fn norm_ws(s: &str) -> String { s.split_whitespace().collect::<Vec<_>>().join(" ") }

// ── Лексер JS ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    /// Строка или часть шаблона (между `${…}`); текст как в исходнике.
    Str(String),
    Ident(String),
    Punct(char),
    Other,
}

/// Токены с номером строки. Комментарии отброшены; шаблон даёт свои части
/// строками, а выражения внутри `${…}` — обычными токенами. Регулярное
/// выражение — по обычному правилу: `/` там, где не может стоять деление.
pub(crate) fn js_tokens(src: &str) -> Vec<(usize, Tok)> {
    let c: Vec<char> = src.chars().collect();
    let mut out: Vec<(usize, Tok)> = Vec::new();
    let mut line = 1;
    let mut i = 0;
    // Стек: true — внутри `${…}` шаблона (глубина фигурных скобок рядом).
    let mut tpl_depth: Vec<usize> = Vec::new();
    let mut brace = 0usize;

    // Шаблон с позиции `start` (после ` или после } закрывающей `${`).
    fn template(c: &[char], mut i: usize, line: &mut usize, out: &mut Vec<(usize, Tok)>) -> (usize, bool) {
        let mut s = String::new();
        let l0 = *line;
        while i < c.len() {
            match c[i] {
                '\\' => { s.push(c[i]); if i + 1 < c.len() { s.push(c[i + 1]); if c[i + 1] == '\n' { *line += 1; } } i += 2; }
                '`' => { out.push((l0, Tok::Str(s))); return (i + 1, false); }
                '$' if c.get(i + 1) == Some(&'{') => { out.push((l0, Tok::Str(s))); return (i + 2, true); }
                ch => { if ch == '\n' { *line += 1; } s.push(ch); i += 1; }
            }
        }
        out.push((l0, Tok::Str(s)));
        (i, false)
    }

    let regex_allowed = |out: &Vec<(usize, Tok)>| match out.last().map(|t| &t.1) {
        None => true,
        Some(Tok::Ident(w)) => ["return", "typeof", "case", "do", "else", "in", "of", "new", "delete", "void", "throw"].contains(&w.as_str()),
        Some(Tok::Punct(p)) => !matches!(p, ')' | ']' | '}'),
        Some(Tok::Str(_)) | Some(Tok::Other) => false,
    };

    while i < c.len() {
        let ch = c[i];
        if ch == '\n' { line += 1; i += 1; continue; }
        if ch.is_whitespace() { i += 1; continue; }
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' { i += 1; }
            continue;
        }
        if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') { if c[i] == '\n' { line += 1; } i += 1; }
            i += 2;
            continue;
        }
        if ch == '\'' || ch == '"' {
            let mut s = String::new();
            i += 1;
            while i < c.len() && c[i] != ch {
                if c[i] == '\\' { s.push(c[i]); i += 1; }
                if i < c.len() { s.push(c[i]); i += 1; }
            }
            i += 1;
            out.push((line, Tok::Str(s)));
            continue;
        }
        if ch == '`' {
            let (ni, open) = template(&c, i + 1, &mut line, &mut out);
            i = ni;
            if open { tpl_depth.push(brace); brace += 1; }
            continue;
        }
        if ch == '/' && regex_allowed(&out) {
            i += 1;
            let mut class = false;
            while i < c.len() && c[i] != '\n' {
                match c[i] {
                    '\\' => i += 1,
                    '[' => class = true,
                    ']' => class = false,
                    '/' if !class => break,
                    _ => {}
                }
                i += 1;
            }
            i += 1;
            while i < c.len() && c[i].is_ascii_alphabetic() { i += 1; }
            out.push((line, Tok::Other));
            continue;
        }
        if ch.is_alphabetic() || ch == '_' || ch == '$' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') { i += 1; }
            out.push((line, Tok::Ident(c[st..i].iter().collect())));
            continue;
        }
        if ch.is_ascii_digit() {
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '.' || c[i] == '_') { i += 1; }
            out.push((line, Tok::Other));
            continue;
        }
        if ch == '{' { brace += 1; }
        if ch == '}' {
            brace = brace.saturating_sub(1);
            if tpl_depth.last() == Some(&brace) {
                tpl_depth.pop();
                let (ni, open) = template(&c, i + 1, &mut line, &mut out);
                i = ni;
                if open { tpl_depth.push(brace); brace += 1; }
                continue;
            }
        }
        out.push((line, Tok::Punct(ch)));
        i += 1;
    }
    out
}

/// Номера строк с пометкой `// i18n: <что>` — исключения сторожей.
pub(crate) fn marked_lines(src: &str, what: &str) -> std::collections::HashSet<usize> {
    let mark = format!("// i18n: {what}");
    src.lines().enumerate().filter(|(_, l)| l.contains(&mark)).map(|(n, _)| n + 1).collect()
}

/// Ключи вызовов t('…') в app.js с номером строки. Ключ не литералом —
/// ошибка: по литералу его находят тесты и человек. Исключение — строки с
/// пометкой `// i18n: ключ из разметки` (применение data-i18n).
pub(crate) fn js_keys(src: &str) -> Result<Vec<(usize, String)>, String> {
    let toks = js_tokens(src);
    let from_markup = marked_lines(src, "ключ из разметки");
    let mut keys = Vec::new();
    for w in 0..toks.len() {
        let (line, Tok::Ident(name)) = &toks[w] else { continue };
        if name != "t" || from_markup.contains(line) { continue; }
        let prev = w.checked_sub(1).map(|p| &toks[p].1);
        if matches!(prev, Some(Tok::Punct('.'))) { continue; }
        if matches!(prev, Some(Tok::Ident(k)) if k == "function") { continue; }
        if toks.get(w + 1).map(|t| &t.1) != Some(&Tok::Punct('(')) {
            return Err(format!("app.js:{line}: t не вызов — локальная переменная затеняет функцию перевода?"));
        }
        match toks.get(w + 2).map(|t| &t.1) {
            Some(Tok::Str(k)) if matches!(toks.get(w + 3).map(|t| &t.1), Some(Tok::Punct(',' | ')'))) => keys.push((*line, k.clone())),
            _ => return Err(format!("app.js:{line}: ключ t() не литерал")),
        }
    }
    Ok(keys)
}

/// Ключи из Rust: t!("…"), t_log!("…") и t_msg!("…") вне комментариев и тестов.
pub(crate) fn rust_keys() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = root().join("src");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
    files.sort();
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if name == "i18n_audit.rs" { continue; }
        let text = std::fs::read_to_string(&p).unwrap();
        // Отрезается модуль тестов; одиночный #[cfg(test)] у объявления
        // модуля (mod i18n_audit в main.rs) границей не считается.
        let code = text.split("#[cfg(test)]\nmod tests").next().unwrap_or("");
        for (n, l) in code.lines().enumerate() {
            if l.trim_start().starts_with("//") { continue; }
            for m in ["t!(\"", "t_log!(\"", "t_msg!(\""] {
                let mut rest = l;
                while let Some(pos) = rest.find(m) {
                    let before = rest[..pos].chars().last();
                    rest = &rest[pos + m.len()..];
                    if before.is_some_and(|b| b.is_alphanumeric() || b == '_') { continue; }
                    if let Some(end) = rest.find('"') {
                        out.push((format!("{name}:{}", n + 1), rest[..end].to_string()));
                    }
                }
            }
        }
    }
    out
}

// ── Разметка ─────────────────────────────────────────────────────────────────

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let mut from = 0;
    while let Some(pos) = tag[from..].find(&pat) {
        let at = from + pos;
        let ok = at > 0 && tag[..at].ends_with(char::is_whitespace);
        let start = at + pat.len();
        let end = start + tag[start..].find('"')?;
        if ok { return Some(&tag[start..end]); }
        from = end;
    }
    None
}

/// (ключ, текст) из index.html и список нарушений: кириллица без ключа.
pub(crate) fn html_keys(html: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut s = html.to_string();
    while let Some(a) = s.find("<!--") {
        let b = s[a..].find("-->").map(|x| a + x + 3).unwrap_or(s.len());
        s.replace_range(a..b, "");
    }
    let mut keys = Vec::new();
    let mut bad = Vec::new();
    let mut prev_open: Option<String> = None;
    let mut rest = s.as_str();
    while !rest.is_empty() {
        let lt = rest.find('<').unwrap_or(rest.len());
        let text = norm_ws(&rest[..lt]);
        if !text.is_empty() {
            match prev_open.as_deref().and_then(|t| attr(t, "data-i18n")) {
                Some(k) => keys.push((k.to_string(), text)),
                None if has_cyrillic(&text) => bad.push(format!("текст без data-i18n: «{text}»")),
                None => {}
            }
        }
        if lt >= rest.len() { break; }
        let gt = rest[lt..].find('>').map(|x| lt + x + 1).unwrap_or(rest.len());
        let tag = &rest[lt..gt];
        for a in ["title", "placeholder"] {
            if let Some(v) = attr(tag, a) {
                match attr(tag, &format!("data-i18n-{a}")) {
                    Some(k) => keys.push((k.to_string(), norm_ws(v))),
                    None if has_cyrillic(v) => bad.push(format!("{a} без data-i18n-{a}: {tag}")),
                    None => {}
                }
            }
        }
        prev_open = (!tag.starts_with("</")).then(|| tag.to_string());
        rest = &rest[gt..];
    }
    (keys, bad)
}

// ── Тесты ────────────────────────────────────────────────────────────────────

/// Текст разметки с кириллицей — только в элементах с data-i18n (подсказки —
/// с data-i18n-title), и он совпадает со словарём с точностью до пробелов:
/// разметку видно до применения словаря, расходиться с ним она не должна.
#[test]
fn html_text_is_keyed_and_matches_dictionary() {
    let d = dict();
    let (keys, mut bad) = html_keys(&read("../ui/index.html"));
    for (k, text) in &keys {
        match d.get(k).and_then(Value::as_str) {
            Some(v) if norm_ws(v) == *text => {}
            Some(v) => bad.push(format!("{k}: в разметке «{text}», в словаре «{v}»")),
            None => bad.push(format!("{k}: нет в словаре")),
        }
    }
    assert!(bad.is_empty(), "index.html:\n{}", bad.join("\n"));
}

/// Каждый ключ из app.js, index.html и Rust есть в ru.json, и ключи t() —
/// литералы. Сторож на будущее для ключей, которых ещё нет: краснеет на
/// ключе, которого нет в словаре.
#[test]
fn used_keys_exist_in_dictionary() {
    let d = dict();
    let mut missing = Vec::new();
    let js = js_keys(&read("../ui/app.js")).unwrap_or_else(|e| panic!("{e}"));
    for (line, k) in js { if !d.contains_key(&k) { missing.push(format!("app.js:{line}: {k}")); } }
    for (k, _) in html_keys(&read("../ui/index.html")).0 {
        if !d.contains_key(&k) { missing.push(format!("index.html: {k}")); }
    }
    for (place, k) in rust_keys() { if !d.contains_key(&k) { missing.push(format!("{place}: {k}")); } }
    assert!(missing.is_empty(), "ключей нет в ru.json:\n{}", missing.join("\n"));
}

/// В app.js нет ничего с именем t, кроме функции перевода: локальная
/// переменная t затенила бы её, и вызов t('…') в той же области упал бы.
#[test]
fn no_local_t_in_app_js() {
    if let Err(e) = js_keys(&read("../ui/app.js")) { panic!("{e}"); }
}

#[test]
fn js_lexer_skips_strings_comments_and_regex() {
    let src = "// t = 1\nconst a = 't', b = /t[/]x/g; /* t */ const c = `x${t('k', { n })}y`;\nt('z');";
    let keys = js_keys(src).unwrap();
    assert_eq!(keys, vec![(2, "k".to_string()), (3, "z".to_string())]);
    assert!(js_keys("const t = 1;").is_err(), "локальная t");
    assert!(js_keys("items.map(t => t.id);").is_err(), "параметр t");
    assert!(js_keys("t(key);").is_err(), "ключ не литерал");
    assert!(js_keys("x.t = 1; function t(key) {}").is_ok(), "свойство и сама функция");
    assert!(js_keys("t(el.key); // i18n: ключ из разметки").is_ok(), "пометка");
}

// ── Сторожа: кириллица в строках — только в словаре ─────────────────────────

/// Строки app.js с кириллицей вне журнала (аргументы logUi, logTiming и console.*),
/// окна «Сверка с базой» (_verifyState, runVerifyState) и строк с пометкой
/// `// i18n: не переводится`.
pub(crate) fn js_cyrillic(src: &str) -> Vec<(usize, String)> {
    let toks = js_tokens(src);
    let allowed = marked_lines(src, "не переводится");
    // Конец группы, открытой toks[open] (`(` или `{`).
    let group_end = |open: usize| -> usize {
        let (o, c) = match toks[open].1 { Tok::Punct('(') => ('(', ')'), _ => ('{', '}') };
        let mut depth = 0;
        for j in open..toks.len() {
            match toks[j].1 {
                Tok::Punct(p) if p == o => depth += 1,
                Tok::Punct(p) if p == c => { depth -= 1; if depth == 0 { return j + 1; } }
                _ => {}
            }
        }
        toks.len()
    };
    let at = |j: usize| toks.get(j).map(|t| &t.1);
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i].1 {
            Tok::Ident(n) if (n == "logUi" || n == "logTiming") && at(i + 1) == Some(&Tok::Punct('(')) => {
                i = group_end(i + 1); continue;
            }
            Tok::Ident(n) if n == "console" && at(i + 1) == Some(&Tok::Punct('.'))
                && at(i + 3) == Some(&Tok::Punct('(')) => {
                i = group_end(i + 3); continue;
            }
            Tok::Ident(n) if n == "function"
                && matches!(at(i + 1), Some(Tok::Ident(f)) if f == "_verifyState" || f == "runVerifyState") => {
                match (i..toks.len()).find(|&j| toks[j].1 == Tok::Punct('{')) {
                    Some(body) => { i = group_end(body); continue; }
                    None => break,
                }
            }
            Tok::Str(s) if has_cyrillic(s) && !allowed.contains(&toks[i].0) => out.push((toks[i].0, s.clone())),
            _ => {}
        }
        i += 1;
    }
    out
}

/// Строковые литералы Rust с кириллицей вне журнала: пропускаются doc-
/// комментарии, элементы под `#[cfg(test)]`, аргументы `logger::log(…)` и
/// строки с пометкой `// i18n: не переводится`.
pub(crate) fn rs_cyrillic(src: &str) -> Vec<(usize, String)> {
    use proc_macro2::{Delimiter, TokenStream, TokenTree};
    fn walk(ts: TokenStream, allowed: &std::collections::HashSet<usize>, out: &mut Vec<(usize, String)>) {
        let toks: Vec<TokenTree> = ts.into_iter().collect();
        let mut i = 0;
        while i < toks.len() {
            match &toks[i] {
                // #[doc = "…"] — комментарий; #[cfg(test)] — следующий элемент целиком
                TokenTree::Punct(p) if p.as_char() == '#' => {
                    // `//!` — это `#![doc = "…"]`
                    let bang = matches!(toks.get(i + 1), Some(TokenTree::Punct(b)) if b.as_char() == '!');
                    if let Some(TokenTree::Group(g)) = toks.get(i + 1 + bang as usize) {
                        let inner = g.stream().to_string().replace(' ', "");
                        if inner.starts_with("doc") { i += 2 + bang as usize; continue; }
                        if inner == "cfg(test)" {
                            i += 2;
                            while i < toks.len() {
                                let end = matches!(&toks[i], TokenTree::Group(g) if g.delimiter() == Delimiter::Brace)
                                    || matches!(&toks[i], TokenTree::Punct(p) if p.as_char() == ';');
                                i += 1;
                                if end { break; }
                            }
                            continue;
                        }
                    }
                }
                // logger::log(…) — журнал, не переводится
                TokenTree::Ident(id) if id == "logger"
                    && matches!(toks.get(i + 3), Some(TokenTree::Ident(f)) if f == "log")
                    && matches!(toks.get(i + 4), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis) => {
                    i += 5; continue;
                }
                TokenTree::Group(g) => walk(g.stream(), allowed, out),
                TokenTree::Literal(l) => {
                    let text = l.to_string();
                    let line = l.span().start().line;
                    if has_cyrillic(&text) && !allowed.contains(&line) { out.push((line, text)); }
                }
                _ => {}
            }
            i += 1;
        }
    }
    let allowed = marked_lines(src, "не переводится");
    let ts: TokenStream = src.parse().expect("файл Rust не разобрался лексером");
    let mut out = Vec::new();
    walk(ts, &allowed, &mut out);
    out
}

fn rs_sources() -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(root().join("src")).unwrap().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
    files.sort();
    files.into_iter()
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .filter(|(n, _)| n != "i18n_audit.rs")
        .collect()
}

/// В app.js строки с кириллицей — только в журнале и «Сверке с базой»:
/// всё, что видит человек, идёт через t().
#[test]
fn no_cyrillic_literals_in_app_js() {
    let bad: Vec<String> = js_cyrillic(&read("../ui/app.js")).into_iter()
        .map(|(l, s)| format!("app.js:{l}: {s}")).collect();
    assert!(bad.is_empty(), "кириллица вне словаря:\n{}", bad.join("\n"));
}

/// То же для Rust: видимое человеку — через t!/t_msg!.
#[test]
fn no_cyrillic_literals_in_rust() {
    let mut bad = Vec::new();
    for (name, text) in rs_sources() {
        for (l, s) in rs_cyrillic(&text) { bad.push(format!("{name}:{l}: {s}")); }
    }
    assert!(bad.is_empty(), "кириллица вне словаря:\n{}", bad.join("\n"));
}

/// Каждый ключ ru.json (кроме _meta) где-то используется: мёртвый ключ —
/// лишняя работа переводчику и признак забытой правки.
#[test]
fn no_unused_keys() {
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    used.extend(js_keys(&read("../ui/app.js")).unwrap_or_else(|e| panic!("{e}")).into_iter().map(|(_, k)| k));
    used.extend(html_keys(&read("../ui/index.html")).0.into_iter().map(|(k, _)| k));
    used.extend(rust_keys().into_iter().map(|(_, k)| k));
    let unused: Vec<String> = dict().keys().filter(|k| *k != "_meta" && !used.contains(*k)).cloned().collect();
    assert!(unused.is_empty(), "ключи ru.json без использования:\n{}", unused.join("\n"));
}

#[test]
fn cyrillic_scanners_skip_log_and_tests() {
    let js = "logUi(`журнал ${x}`); logTiming('замер', 1); console.error('ошибка'); function _verifyState() { a('сверка'); }\n\
              t('k'); const a = 'видно';\nconst b = 'тоже'; // i18n: не переводится — легаси";
    assert_eq!(js_cyrillic(js), vec![(2, "видно".to_string())]);
    let rs = "//! модуль «текст»\n/// документ «текст»\nfn f() { logger::log(&format!(\"журнал {}\", \"тоже\")); let a = \"видно\"; }\n\
              const B: &str = \"легаси\"; // i18n: не переводится\n#[cfg(test)]\nmod tests { fn g() { let _ = \"тест\"; } }";
    let got: Vec<String> = rs_cyrillic(rs).into_iter().map(|(_, s)| s).collect();
    assert_eq!(got, vec!["\"видно\"".to_string()]);
}
