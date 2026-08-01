// ── Локальный релей для прокси с авторизацией ────────────────────────────────
//
// Chromium игнорирует логин и пароль в `--proxy-server`, поэтому headless-браузеру
// отдаётся адрес этого релея на 127.0.0.1 (без авторизации), а он уже ходит на
// внешний прокси, подставляя `Proxy-Authorization`.
//
// Правила модуля:
// * ошибки глушим молча — релей не имеет права ронять программу;
// * `eprintln!` не используем: в GUI-сборке он не виден (см. CLAUDE.md);
// * настройки читаем на КАЖДОЕ соединение — смена в диалоге подхватывается
//   без перезапуска программы.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

use crate::logger;
use crate::{proxy_cfg_from_settings, strip_proxy_scheme, ProxyCfg};

/// Порт релея. `None` — поднять не удалось; повторных попыток нет.
static RELAY_PORT: OnceLock<Option<u16>> = OnceLock::new();

/// Потолок на голову запроса — защита от бесконечного чтения.
const MAX_HEAD: usize = 32 * 1024;
/// Таймаут чтения/записи: достаточно велик для медленной страницы и достаточно
/// мал, чтобы зависший туннель не держал поток вечно.
const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// Таймаут установки соединения с внешним прокси.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// ── base64 ───────────────────────────────────────────────────────────────────

/// base64 без зависимостей — нужен ровно для `user:pass` в заголовке
/// `Proxy-Authorization: Basic ...`.
pub fn base64_encode(input: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[((n >> 18) & 63) as usize] as char);
        out.push(TBL[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TBL[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[(n & 63) as usize] as char } else { '=' });
    }
    out
}

// ── Подъём релея ─────────────────────────────────────────────────────────────

/// Порт локального релея; поднимает слушателя при первом вызове.
///
/// `None` — поднять не удалось (и уже не получится: результат кэшируется
/// в `OnceLock` на весь процесс). Вызывающий в этом случае должен обойтись
/// без прокси, а не считать ситуацию фатальной.
pub fn ensure_relay() -> Option<u16> {
    *RELAY_PORT.get_or_init(|| {
        // Порт 0 — ОС сама выдаёт свободный. Никаких констант: жёсткий порт
        // означал бы тихий отказ при занятости, как у SERVER_PORT расширения.
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();

        std::thread::Builder::new()
            .name("proxy-relay".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    let Ok(client) = conn else { continue };
                    // Поток на соединение: браузер открывает их пачкой,
                    // последовательная обработка застопорила бы загрузку страницы.
                    let _ = std::thread::Builder::new()
                        .name("proxy-relay-conn".into())
                        .spawn(move || { let _ = handle_conn(client); });
                }
            })
            .ok()?;

        Some(port)
    })
}

// ── Обработка соединения ─────────────────────────────────────────────────────

fn handle_conn(mut client: TcpStream) -> Option<()> {
    client.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    client.set_write_timeout(Some(IO_TIMEOUT)).ok()?;

    // Настройки читаем здесь, а не при старте релея — смена прокси в диалоге
    // подхватывается со следующего соединения, без перезапуска программы.
    let Some(cfg) = proxy_cfg_from_settings() else {
        logger::log("релей: соединение закрыто — прокси выключен в настройках");
        return None;
    };
    // Релей нужен ТОЛЬКО для прокси с логином: без него do_screenshot указывает
    // браузеру внешний прокси напрямую, и звать нас незачем. Сокет закроется
    // при выходе из функции.
    if cfg.user.is_empty() {
        logger::log("релей: соединение закрыто — прокси без логина, релей не нужен");
        return None;
    }

    let head = read_head(&mut client)?;
    let line_end = find(&head, b"\r\n")?;
    let first = String::from_utf8_lossy(&head[..line_end]).into_owned();
    let mut parts = first.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;

    // В журнал — только метод и цель. Голову целиком писать НЕЛЬЗЯ: в ней
    // заголовок Proxy-Authorization с паролем пользователя.
    logger::log(&format!("релей: {method} {target}"));

    let Some(mut upstream) = connect_upstream(&cfg) else {
        logger::log(&format!("релей: не удалось подключиться к прокси {}:{}",
            strip_proxy_scheme(&cfg.host), cfg.port));
        return None;
    };

    let creds = format!("{}:{}", cfg.user, cfg.pass);
    let auth  = format!("Proxy-Authorization: Basic {}\r\n", base64_encode(creds.as_bytes()));

    if method.eq_ignore_ascii_case("CONNECT") {
        // https-страница: просим у внешнего прокси туннель на host:port.
        let req = format!(
            "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{auth}Proxy-Connection: keep-alive\r\n\r\n"
        );
        upstream.write_all(req.as_bytes()).ok()?;
        upstream.flush().ok()?;

        let resp = read_head(&mut upstream)?;
        if !status_is_2xx(&resp) {
            // 407 сюда и приходит: прокси отверг наши учётные данные.
            logger::log(&format!("релей: прокси ответил на CONNECT кодом {}",
                status_code(&resp).map(|c| c.to_string()).unwrap_or_else(|| "?".into())));
            return None;
        }
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").ok()?;
        client.flush().ok()?;
    } else {
        // Обычный запрос с абсолютным URL: пересылаем голову как есть, выкинув
        // чужой Proxy-Authorization и подставив свой сразу после строки запроса.
        let mut out = Vec::with_capacity(head.len() + auth.len());
        for (i, line) in split_lines(&head).into_iter().enumerate() {
            if i == 0 {
                out.extend_from_slice(line);
                out.extend_from_slice(b"\r\n");
                out.extend_from_slice(auth.as_bytes());
                continue;
            }
            if line.is_empty() || starts_with_ci(line, b"proxy-authorization:") {
                continue;
            }
            out.extend_from_slice(line);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        upstream.write_all(&out).ok()?;
        upstream.flush().ok()?;
    }

    pump(client, upstream);
    Some(())
}

fn connect_upstream(cfg: &ProxyCfg) -> Option<TcpStream> {
    // Схему срезаем той же функцией, что и reqwest-путь: адрес обязан
    // разбираться одинаково, иначе релей пойдёт не туда, куда клиент.
    let host = strip_proxy_scheme(&cfg.host);
    let addr = format!("{host}:{}", cfg.port).to_socket_addrs().ok()?.next()?;
    let s = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    s.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    s.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    Some(s)
}

/// Двунаправленное копирование до закрытия любой из сторон.
fn pump(client: TcpStream, upstream: TcpStream) {
    let (Ok(c2), Ok(u2)) = (client.try_clone(), upstream.try_clone()) else { return };

    let t = std::thread::spawn(move || {
        let (mut r, mut w) = (c2, u2);
        let _ = std::io::copy(&mut r, &mut w);
        let _ = w.shutdown(Shutdown::Write);
        let _ = r.shutdown(Shutdown::Read);
    });

    {
        let (mut r, mut w) = (upstream, client);
        let _ = std::io::copy(&mut r, &mut w);
        let _ = w.shutdown(Shutdown::Write);
        let _ = r.shutdown(Shutdown::Read);
    }
    let _ = t.join();
}

// ── Разбор HTTP ──────────────────────────────────────────────────────────────

/// Прочитать голову запроса/ответа побайтно до `\r\n\r\n` включительно.
///
/// Побайтно намеренно: буферизованное чтение утащило бы в свой буфер начало
/// тела (а для CONNECT — начало TLS-рукопожатия), после чего сырое копирование
/// потеряло бы эти байты и соединение сломалось бы.
fn read_head(s: &mut TcpStream) -> Option<Vec<u8>> {
    let mut buf = Vec::with_capacity(1024);
    let mut b = [0u8; 1];
    loop {
        match s.read(&mut b) {
            Ok(0) => return None,
            Ok(_) => {
                buf.push(b[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    return Some(buf);
                }
                if buf.len() > MAX_HEAD {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

fn status_code(head: &[u8]) -> Option<u16> {
    let line = match find(head, b"\r\n") { Some(i) => &head[..i], None => head };
    String::from_utf8_lossy(line)
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
}

fn status_is_2xx(head: &[u8]) -> bool {
    status_code(head).map(|c| (200..300).contains(&c)).unwrap_or(false)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn split_lines(head: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(i) = find(&head[start..], b"\r\n") {
        out.push(&head[start..start + i]);
        start += i + 2;
    }
    out
}

fn starts_with_ci(hay: &[u8], prefix: &[u8]) -> bool {
    hay.len() >= prefix.len() && hay[..prefix.len()].eq_ignore_ascii_case(prefix)
}
