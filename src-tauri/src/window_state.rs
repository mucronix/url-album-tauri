// ── Размер и положение окна между запусками ─────────────────────────────────
//
// Хранится в settings.json, ключ `window`: обычный (не развёрнутый) прямоугольник
// окна в экранных физических пикселях и признак «развёрнуто». Не плагин
// window-state: тот пишет в AppData, а программа портативная.
//
// Читается и пишется через GetWindowPlacement / SetWindowPlacement: они знают
// обычный прямоугольник и у развёрнутого или свёрнутого окна, поэтому размер
// «до разворота» не теряется. Их координаты — относительно рабочей области
// основного монитора (workspace), а храним экранные: переводит `ws_offset`.

/// Прямоугольник в физических пикселях.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

/// Что лежит в settings.json под ключом `window`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Saved {
    pub x: i32, pub y: i32, pub w: i32, pub h: i32,
    #[serde(default)]
    pub maximized: bool,
}

/// Монитор: рабочая область (без панели задач) и масштаб.
#[derive(Clone, Copy, Debug)]
pub struct Screen { pub work: Rect, pub scale: f64 }

/// Окно по умолчанию, логические пиксели. Было 1200×750 — при масштабе
/// 150 % на ноутбуке не влезало (greeple).
pub const DEFAULT_W: f64 = 1000.0;
pub const DEFAULT_H: f64 = 650.0;

/// Полоса заголовка, за которую окно можно взять: сохранённое положение
/// годится, если на каком-то экране видно хотя бы TITLE_VISIBLE_W×TITLE_H её пикселей.
const TITLE_H: i32 = 30;
const TITLE_VISIBLE_W: i32 = 100;

/// Сохранённое — из settings.json; нет, не разобралось или нелепое — None.
pub fn saved_from_settings(raw: &str) -> Option<Saved> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let s: Saved = serde_json::from_value(v.get("window")?.clone()).ok()?;
    (s.w > 0 && s.h > 0 && s.w < 100_000 && s.h < 100_000).then_some(s)
}

/// Окно по умолчанию: 1000×650 логических, не больше 90 % рабочей области, по центру.
pub fn default_rect(s: &Screen) -> Rect {
    let w = ((DEFAULT_W * s.scale) as i32).min((s.work.w as f64 * 0.9) as i32);
    let h = ((DEFAULT_H * s.scale) as i32).min((s.work.h as f64 * 0.9) as i32);
    Rect { x: s.work.x + (s.work.w - w) / 2, y: s.work.y + (s.work.h - h) / 2, w, h }
}

/// Пересечение по горизонтали и вертикали, пикселей (0 — не пересекаются).
fn overlap(a: &Rect, b: &Rect) -> (i32, i32) {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    (w.max(0), h.max(0))
}

/// Где поставить окно и развернуть ли. Сохранённое — там же, если его
/// заголовок виден на одном из экранов (иначе за окно не взяться: второй
/// монитор отключили, сменили разрешение) — тогда по умолчанию на основном.
/// Больше рабочей области своего экрана — ужимается до неё.
pub fn placement(saved: Option<Saved>, screens: &[Screen], primary: &Screen) -> (Rect, bool) {
    let Some(s) = saved else { return (default_rect(primary), false) };
    let title = Rect { x: s.x, y: s.y, w: s.w, h: TITLE_H };
    let home = screens.iter().find(|m| {
        let (w, h) = overlap(&title, &m.work);
        w >= TITLE_VISIBLE_W && h >= TITLE_H / 2
    });
    match home {
        Some(m) => (Rect { x: s.x, y: s.y, w: s.w.min(m.work.w), h: s.h.min(m.work.h) }, s.maximized),
        None => (default_rect(primary), s.maximized),
    }
}

// ── Win32 ────────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod win {
    #[repr(C)] #[derive(Default)] struct Point { x: i32, y: i32 }
    #[repr(C)] #[derive(Default)] struct WinRect { left: i32, top: i32, right: i32, bottom: i32 }
    #[repr(C)] #[derive(Default)]
    struct WindowPlacement {
        length: u32, flags: u32, show_cmd: u32,
        min_pos: Point, max_pos: Point, normal: WinRect,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetWindowPlacement(hwnd: isize, wp: *mut WindowPlacement) -> i32;
        fn SetWindowPlacement(hwnd: isize, wp: *const WindowPlacement) -> i32;
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    }
    const SW_HIDE: u32 = 0;
    const SW_SHOWMINIMIZED: u32 = 2;
    const SW_SHOWMAXIMIZED: u32 = 3;
    const WPF_RESTORETOMAXIMIZED: u32 = 2;

    /// Обычный прямоугольник и «развёрнуто» — в экранных координатах
    /// (`ws_offset` — сдвиг рабочей области основного монитора).
    pub fn read(hwnd: isize, ws_offset: (i32, i32)) -> Option<super::Saved> {
        let mut wp = WindowPlacement { length: std::mem::size_of::<WindowPlacement>() as u32, ..Default::default() };
        if unsafe { GetWindowPlacement(hwnd, &mut wp) } == 0 { return None; }
        let r = &wp.normal;
        let maximized = wp.show_cmd == SW_SHOWMAXIMIZED
            || (wp.show_cmd == SW_SHOWMINIMIZED && wp.flags & WPF_RESTORETOMAXIMIZED != 0);
        Some(super::Saved { x: r.left + ws_offset.0, y: r.top + ws_offset.1,
                            w: r.right - r.left, h: r.bottom - r.top, maximized })
    }

    /// Поставить скрытое окно на место; показывает его `show`.
    pub fn apply(hwnd: isize, r: super::Rect, ws_offset: (i32, i32)) -> bool {
        let wp = WindowPlacement {
            length: std::mem::size_of::<WindowPlacement>() as u32, flags: 0, show_cmd: SW_HIDE,
            min_pos: Point { x: -1, y: -1 }, max_pos: Point { x: -1, y: -1 },
            normal: WinRect { left: r.x - ws_offset.0, top: r.y - ws_offset.1,
                              right: r.x - ws_offset.0 + r.w, bottom: r.y - ws_offset.1 + r.h },
        };
        unsafe { SetWindowPlacement(hwnd, &wp) != 0 }
    }

    pub fn show_maximized(hwnd: isize) {
        unsafe { ShowWindow(hwnd, SW_SHOWMAXIMIZED as i32); }
    }
}

#[cfg(windows)]
pub use win::{apply, read, show_maximized};

#[cfg(test)]
mod tests {
    use super::*;

    const FHD: Screen = Screen { work: Rect { x: 0, y: 0, w: 1920, h: 1040 }, scale: 1.0 };
    /// Ноутбук 1366×768, масштаб 150 %: рабочая область ≈ 910×485 логических.
    const LAPTOP_150: Screen = Screen { work: Rect { x: 0, y: 0, w: 1366, h: 728 }, scale: 1.5 };
    const RIGHT: Screen = Screen { work: Rect { x: 1920, y: 0, w: 1920, h: 1040 }, scale: 1.0 };

    fn saved(x: i32, y: i32, w: i32, h: i32, maximized: bool) -> Saved { Saved { x, y, w, h, maximized } }

    #[test]
    fn default_is_1000x650_centered() {
        assert_eq!(default_rect(&FHD), Rect { x: 460, y: 195, w: 1000, h: 650 });
    }

    /// 1000×650 при 150 % — 1500×975 физических, а рабочая область 1366×728:
    /// окно ужимается до 90 % и встаёт по центру, целиком на экране.
    #[test]
    fn default_fits_work_area_at_150_percent() {
        let r = default_rect(&LAPTOP_150);
        assert_eq!((r.w, r.h), (1229, 655));
        assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= 1366 && r.y + r.h <= 728, "{r:?}");
    }

    #[test]
    fn no_saved_gives_default() {
        assert_eq!(placement(None, &[FHD], &FHD), (default_rect(&FHD), false));
    }

    #[test]
    fn saved_on_visible_screen_is_kept() {
        let s = saved(200, 150, 900, 600, false);
        assert_eq!(placement(Some(s), &[FHD], &FHD), (Rect { x: 200, y: 150, w: 900, h: 600 }, false));
        let s2 = saved(2100, 100, 900, 600, false);
        assert_eq!(placement(Some(s2), &[FHD, RIGHT], &FHD).0, Rect { x: 2100, y: 100, w: 900, h: 600 },
                   "на втором мониторе — там и остаётся");
    }

    /// Второй монитор отключили: окно с него — по центру основного.
    #[test]
    fn saved_on_missing_screen_goes_to_primary_center() {
        let s = saved(2100, 100, 900, 600, false);
        assert_eq!(placement(Some(s), &[FHD], &FHD).0, default_rect(&FHD));
        let far = saved(-5000, 100, 900, 600, false);
        assert_eq!(placement(Some(far), &[FHD], &FHD).0, default_rect(&FHD));
    }

    /// Наполовину за краем, но заголовок виден (за него можно взять) — не трогаем.
    #[test]
    fn partly_visible_is_kept() {
        let s = saved(1920 - 300, 400, 900, 600, false);
        assert_eq!(placement(Some(s), &[FHD], &FHD).0, Rect { x: 1620, y: 400, w: 900, h: 600 });
        let above = saved(200, -580, 900, 600, false);   // заголовок над экраном — не взять
        assert_eq!(placement(Some(above), &[FHD], &FHD).0, default_rect(&FHD));
    }

    #[test]
    fn maximized_is_restored_maximized() {
        let s = saved(200, 150, 900, 600, true);
        assert_eq!(placement(Some(s), &[FHD], &FHD), (Rect { x: 200, y: 150, w: 900, h: 600 }, true));
        let gone = saved(2100, 100, 900, 600, true);
        assert!(placement(Some(gone), &[FHD], &FHD).1, "монитора нет — развёрнутое разворачивается на основном");
    }

    /// Сохранённое больше рабочей области (сменили разрешение) — ужимается до неё.
    #[test]
    fn saved_larger_than_screen_shrinks() {
        let s = saved(0, 0, 3000, 2000, false);
        let r = placement(Some(s), &[FHD], &FHD).0;
        assert_eq!((r.w, r.h), (1920, 1040));
    }

    #[test]
    fn settings_parsing() {
        assert_eq!(saved_from_settings(r#"{"window": {"x": 1, "y": 2, "w": 900, "h": 600, "maximized": true}}"#),
                   Some(saved(1, 2, 900, 600, true)));
        assert_eq!(saved_from_settings(r#"{"window": {"x": 1, "y": 2, "w": 900, "h": 600}}"#),
                   Some(saved(1, 2, 900, 600, false)), "без maximized — не развёрнуто");
        assert_eq!(saved_from_settings(""), None);
        assert_eq!(saved_from_settings("{"), None);
        assert_eq!(saved_from_settings(r#"{"logEnabled": true}"#), None);
        assert_eq!(saved_from_settings(r#"{"window": {"x": 1, "y": 2, "w": 0, "h": 600}}"#), None, "нулевой размер");
        assert_eq!(saved_from_settings(r#"{"window": {"x": 1, "y": 2, "w": -5, "h": 600}}"#), None);
        assert_eq!(saved_from_settings(r#"{"window": "big"}"#), None);
    }
}
