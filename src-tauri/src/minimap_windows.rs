//! Visible-desktop GDI capture for windowed/borderless PoE 2. No client hooks.
//! Revalidate focus, physical client bounds and occlusion before every capture.
use super::{Image, REGION_H, REGION_W, REGION_X};
use std::{ffi::c_void, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::{
        BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
        GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HBITMAP, HDC, HGDIOBJ, SRCCOPY,
    },
    System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::{
        HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        },
        WindowsAndMessaging::{
            EnumWindows, GetClientRect, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
            GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
            SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        },
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

pub struct Region {
    window: HWND,
    bounds: Bounds,
}

struct DpiGuard(DPI_AWARENESS_CONTEXT);
impl DpiGuard {
    fn enter() -> Result<Self, String> {
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.is_null() {
            Err("Could not obtain physical screen coordinates".into())
        } else {
            Ok(Self(previous))
        }
    }
}
impl Drop for DpiGuard {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

fn game_window() -> Result<HWND, String> {
    // Only inspect foreground metadata. Do not open the game for memory access.
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() || unsafe { IsIconic(window) } != 0 {
        return Err("Focus the PoE 2 window to read the minimap".into());
    }
    let mut title = [0u16; 256];
    let length = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
    if length <= 0 || String::from_utf16_lossy(&title[..length as usize]) != "Path of Exile 2" {
        return Err("Minimap paused: PoE 2 is not the active window".into());
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err("Cannot verify the PoE 2 executable".into());
    }
    let mut path = [0u16; 32768];
    let mut length = path.len() as u32;
    let success = unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) };
    unsafe {
        CloseHandle(process);
    }
    let path = String::from_utf16_lossy(&path[..length.min(path.len() as u32) as usize]);
    let filename = path.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
    if success == 0
        || !matches!(
            filename.as_str(),
            "pathofexile.exe"
                | "pathofexilesteam.exe"
                | "pathofexile_x64.exe"
                | "pathofexile_x64steam.exe"
        )
    {
        return Err("Foreground executable is not a supported PoE 2 client".into());
    }
    Ok(window)
}

fn crop_bounds(origin: [i32; 2], size: [i32; 2]) -> Result<Bounds, String> {
    if size[0] < 320 || size[1] < 200 || size[0] > 32768 || size[1] > 32768 {
        return Err("PoE 2 client area is too small or invalid".into());
    }
    let bounds = Bounds {
        x: origin[0] + (size[0] as f64 * REGION_X).round() as i32,
        y: origin[1],
        width: ((size[0] as f64 * REGION_W).round() as i32)
            .min(size[0] - (size[0] as f64 * REGION_X).round() as i32),
        height: (size[1] as f64 * REGION_H).round() as i32,
    };
    if bounds.width as i64 * bounds.height as i64 > 8_000_000 {
        return Err("Minimap crop exceeds capture limit".into());
    }
    Ok(bounds)
}

fn window_bounds(window: HWND) -> Result<Bounds, String> {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    if unsafe { GetClientRect(window, &mut client) } == 0
        || unsafe { ClientToScreen(window, &mut origin) } == 0
    {
        return Err("Cannot locate the PoE 2 client area".into());
    }
    crop_bounds([origin.x, origin.y], [client.right, client.bottom])
}

pub fn locate_minimap() -> Result<Region, String> {
    let _dpi = DpiGuard::enter()?;
    let window = game_window()?;
    Ok(Region {
        window,
        bounds: window_bounds(window)?,
    })
}

fn intersects(a: Bounds, b: RECT) -> bool {
    a.x < b.right && a.x + a.width > b.left && a.y < b.bottom && a.y + a.height > b.top
}

struct Occlusion {
    game: HWND,
    crop: Bounds,
    reached_game: bool,
    covered: bool,
}
unsafe extern "system" fn check_window(window: HWND, parameter: LPARAM) -> i32 {
    let check = unsafe { &mut *(parameter as *mut Occlusion) };
    if window == check.game {
        check.reached_game = true;
        return 0;
    }
    let mut rect = RECT::default();
    if unsafe { IsWindowVisible(window) } != 0
        && unsafe { IsIconic(window) } == 0
        && unsafe { GetWindowRect(window, &mut rect) } != 0
        && intersects(check.crop, rect)
    {
        check.covered = true;
        return 0;
    }
    1
}

pub fn capture(region: &Region) -> Result<Image, String> {
    let _dpi = DpiGuard::enter()?;
    if game_window()? != region.window {
        return Err("Game window changed; locating minimap again".into());
    }
    let bounds = window_bounds(region.window)?;
    if bounds != region.bounds {
        return Err("Game moved or resized; locating minimap again".into());
    }
    let desktop = unsafe {
        Bounds {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    };
    if bounds.x < desktop.x
        || bounds.y < desktop.y
        || bounds.x + bounds.width > desktop.x + desktop.width
        || bounds.y + bounds.height > desktop.y + desktop.height
    {
        return Err("Minimap is partly off screen".into());
    }
    let mut check = Occlusion {
        game: region.window,
        crop: bounds,
        reached_game: false,
        covered: false,
    };
    unsafe {
        EnumWindows(Some(check_window), &mut check as *mut _ as LPARAM);
    }
    if check.covered || !check.reached_game {
        return Err("Minimap covered by another window; capture paused".into());
    }
    let screen = unsafe { GetDC(null_mut()) };
    if screen.is_null() {
        return Err("Desktop capture is unavailable".into());
    }
    // ReleaseDC is required even if bitmap allocation or BitBlt fails.
    struct Screen(HDC);
    impl Drop for Screen {
        fn drop(&mut self) {
            unsafe {
                ReleaseDC(null_mut(), self.0);
            }
        }
    }
    let screen = Screen(screen);
    let image = read_crop(screen.0, bounds)?;
    // Discard pixels if focus/geometry changed during the blit.
    if unsafe { GetForegroundWindow() } != region.window || window_bounds(region.window)? != bounds
    {
        return Err("Game changed during capture; frame discarded".into());
    }
    Ok(image)
}

struct BitmapDc {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut c_void,
}
impl BitmapDc {
    fn new(source: HDC, width: i32, height: i32) -> Result<Self, String> {
        let dc = unsafe { CreateCompatibleDC(source) };
        if dc.is_null() {
            return Err("Could not allocate capture DC".into());
        }
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        };
        let mut bits = null_mut();
        let bitmap =
            unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
        if bitmap.is_null() || bits.is_null() {
            unsafe {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(dc);
            }
            return Err("Could not allocate capture bitmap".into());
        }
        let old = unsafe { SelectObject(dc, bitmap) };
        if old.is_null() || old as isize == -1 {
            unsafe {
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
            return Err("Could not select capture bitmap".into());
        }
        Ok(Self {
            dc,
            bitmap,
            old,
            bits,
        })
    }
}
impl Drop for BitmapDc {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn read_crop(source: HDC, bounds: Bounds) -> Result<Image, String> {
    let target = BitmapDc::new(source, bounds.width, bounds.height)?;
    if unsafe {
        BitBlt(
            target.dc,
            0,
            0,
            bounds.width,
            bounds.height,
            source,
            bounds.x,
            bounds.y,
            SRCCOPY,
        )
    } == 0
    {
        return Err("Screen copy failed; use windowed/borderless mode".into());
    }
    // GDI may batch writes to the DIB; flush before directly accessing its memory.
    if unsafe { windows_sys::Win32::Graphics::Gdi::GdiFlush() } == 0 {
        return Err("Could not finish screen copy".into());
    }
    let length = bounds.width as usize * bounds.height as usize * 4;
    let bgra = unsafe { std::slice::from_raw_parts(target.bits.cast::<u8>(), length) };
    let pixels = bgra
        .chunks_exact(4)
        .flat_map(|p| [p[2], p[1], p[0]])
        .collect();
    Ok(Image {
        width: bounds.width as usize,
        height: bounds.height as usize,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_uses_client_pixels_and_negative_monitor_origins() {
        assert_eq!(
            crop_bounds([-1920, 40], [1920, 1080]).unwrap(),
            Bounds {
                x: -300,
                y: 40,
                width: 300,
                height: 259
            }
        );
        assert_eq!(
            crop_bounds([200, 100], [1280, 720]).unwrap(),
            Bounds {
                x: 1280,
                y: 100,
                width: 200,
                height: 173
            }
        );
        assert!(crop_bounds([0, 0], [0, 0]).is_err());
        let rounded = crop_bounds([0, 0], [1008, 720]).unwrap();
        assert_eq!(rounded.x + rounded.width, 1008);
    }
    #[test]
    fn gdi_crop_reads_top_down_rgb_without_capturing_the_desktop() {
        let source = BitmapDc::new(null_mut(), 4, 3).unwrap();
        let bytes = unsafe { std::slice::from_raw_parts_mut(source.bits.cast::<u8>(), 4 * 3 * 4) };
        for y in 0..3 {
            for x in 0..4 {
                bytes[(y * 4 + x) * 4..][..4].copy_from_slice(&[x as u8, y as u8, 100, 0]);
            }
        }
        for _ in 0..50 {
            let image = read_crop(
                source.dc,
                Bounds {
                    x: 1,
                    y: 1,
                    width: 2,
                    height: 2,
                },
            )
            .unwrap();
            assert_eq!((image.width, image.height), (2, 2));
            assert_eq!(image.pixels, [100, 1, 1, 100, 1, 2, 100, 2, 1, 100, 2, 2]);
        }
    }
    #[test]
    fn touching_window_edges_do_not_occlude_the_crop() {
        let a = Bounds {
            x: -300,
            y: 0,
            width: 300,
            height: 250,
        };
        assert!(!intersects(
            a,
            RECT {
                left: 0,
                top: 0,
                right: 200,
                bottom: 200
            }
        ));
        assert!(intersects(
            a,
            RECT {
                left: -100,
                top: 20,
                right: 200,
                bottom: 200
            }
        ));
    }
}
