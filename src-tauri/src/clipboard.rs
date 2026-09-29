// Чтение текста из буфера обмена Windows через Win32.
//
// navigator.clipboard.readText() в WebView2 показывает запрос разрешения
// («Сайт http://tauri.localhost хочет просматривать текст… из буфера обмена»),
// хотя permissions.query отвечает «granted», — заранее не узнать. Запрос
// забирает фокус, и правка в поле (переименование, заметка) закрывается.
// Поэтому текст из буфера читает Rust: user32/kernel32 уже слинкованы,
// новых зависимостей нет.

use std::ffi::c_void;

const CF_UNICODETEXT: u32 = 13;
/// Попыток открыть буфер и пауза между ними: пока его держит другая программа
/// (копирует в него прямо сейчас), OpenClipboard отказывает. ~100 мс в сумме.
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_PAUSE_MS: u64 = 20;

#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(hwnd: *mut c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalLock(mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(mem: *mut c_void) -> i32;
    fn GlobalSize(mem: *mut c_void) -> usize;
}

/// Закрывает буфер при любом выходе, в том числе раннем: открытый и
/// не закрытый буфер не даст остальным программам в него писать.
struct Opened;
impl Drop for Opened {
    fn drop(&mut self) { unsafe { CloseClipboard(); } }
}

/// Текст из буфера. `Ok(None)` — текста в буфере нет (пусто, картинка, файлы);
/// `Err` — буфер не открылся за все попытки или данные не читаются.
pub fn read_text() -> Result<Option<String>, String> {
    let mut opened = false;
    for attempt in 0..OPEN_ATTEMPTS {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 { opened = true; break; }
        if attempt + 1 < OPEN_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(OPEN_PAUSE_MS));
        }
    }
    if !opened { return Err("буфер обмена занят другой программой".into()); }
    let _guard = Opened;

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 { return Ok(None); }
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() { return Err("не удалось получить текст из буфера обмена".into()); }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() { return Err("не удалось прочитать текст из буфера обмена".into()); }
        let units = std::slice::from_raw_parts(ptr, GlobalSize(handle) / 2);
        let text = utf16_until_nul(units);
        GlobalUnlock(handle);
        Ok(Some(text))
    }
}

/// Строка из блока UTF-16: до первого NUL, а без него — весь блок (размер
/// блока бывает больше строки). Одиночные суррогаты — заменой, а не отказом.
fn utf16_until_nul(units: &[u16]) -> String {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> { s.encode_utf16().collect() }

    #[test]
    fn utf16_block_is_read_until_nul() {
        // Кириллица, латиница, перевод строки Windows
        let mut b = w("Привет, world\r\nвторая");
        b.push(0);
        assert_eq!(utf16_until_nul(&b), "Привет, world\r\nвторая");
        // Хвост после NUL (блок больше строки) не попадает в текст
        let mut b = w("abc");
        b.extend([0, 'x' as u16, 'y' as u16]);
        assert_eq!(utf16_until_nul(&b), "abc");
        // Без NUL — весь блок
        assert_eq!(utf16_until_nul(&w("без нуля")), "без нуля");
        // Пустой буфер и сразу NUL
        assert_eq!(utf16_until_nul(&[]), "");
        assert_eq!(utf16_until_nul(&[0, 'a' as u16]), "");
        // Суррогатная пара (эмодзи) — цела; одиночный суррогат — замена, не паника
        let mut b = w("🙂 ok");
        b.push(0);
        assert_eq!(utf16_until_nul(&b), "🙂 ok");
        assert_eq!(utf16_until_nul(&[0xD800, 'a' as u16, 0]), "\u{FFFD}a");
    }
}
