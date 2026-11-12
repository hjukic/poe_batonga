//! Visible-desktop GDI capture for windowed/borderless PoE 2. No client hooks.
//! Revalidate focus, physical client bounds and occlusion before every capture.
//! Two crops: the corner minimap and, while the minimap is hidden, the big map
//! around the player.
use super::{big_layout, bigmap, scan_layout, Image, Scan, REGION_H, REGION_W};
use std::{ffi::c_void, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::{
        BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
        GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HBITMAP, HDC, HGDIOBJ, SRCCOPY,
    },
    System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
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
    /// The big-map crop around the player.
    big: Bounds,
    /// The whole client area, read now and then for map icons far from the player.
    whole: Bounds,
    /// Where the minimap draws the player, inside the minimap crop, in pixels.
    pub minimap_marker: [f64; 2],
    pub plan: bigmap::Plan,
    pub scan: Scan,
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
    // Anchored to the right edge and sized by height, as the game draws its minimap.
    let width = ((size[1] as f64 * REGION_W).round() as i32).min(size[0]);
    let bounds = Bounds {
        x: origin[0] + size[0] - width,
        y: origin[1],
        width,
        height: (size[1] as f64 * REGION_H).round() as i32,
    };
    if bounds.width as i64 * bounds.height as i64 > 8_000_000 {
        return Err("Minimap crop exceeds capture limit".into());
    }
    Ok(bounds)
}

/// The big-map crop for a client area at `origin` of `size` whose minimap crop is
/// `minimap`, where the minimap draws the player inside that crop, how to read it, and
/// how to read the whole client area.
fn big_crop(origin: [i32; 2], size: [i32; 2], minimap: Bounds) -> (Bounds, [f64; 2], bigmap::Plan, Scan) {
    let window = [size[0] as f64, size[1] as f64];
    let minimap_origin = [(minimap.x - origin[0]) as f64, (minimap.y - origin[1]) as f64];
    let layout = big_layout(window, minimap_origin, [minimap.width as usize, minimap.height as usize]);
    let [x, y, w, h] = layout.area;
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (x1, y1) = ((x + w).ceil() as i32, (y + h).ceil() as i32);
    let crop = Bounds { x: origin[0] + x0, y: origin[1] + y0, width: x1 - x0, height: y1 - y0 };
    let marker = [layout.marker[0] - x0 as f64, layout.marker[1] - y0 as f64];
    let plan = bigmap::Plan { marker, radius: bigmap::radius(window), frame: layout.frame };
    let scan = scan_layout(window, [0.0, 0.0], [size[0] as usize, size[1] as usize], &layout);
    (crop, layout.minimap_marker, plan, scan)
}

fn client_area(window: HWND) -> Result<([i32; 2], [i32; 2]), String> {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    if unsafe { GetClientRect(window, &mut client) } == 0
        || unsafe { ClientToScreen(window, &mut origin) } == 0
    {
        return Err("Cannot locate the PoE 2 client area".into());
    }
    Ok(([origin.x, origin.y], [client.right, client.bottom]))
}

fn window_bounds(window: HWND) -> Result<Bounds, String> {
    let (origin, size) = client_area(window)?;
    crop_bounds(origin, size)
}

pub fn locate_minimap() -> Result<Region, String> {
    let _dpi = DpiGuard::enter()?;
    let window = game_window()?;
    let (origin, size) = client_area(window)?;
    let bounds = crop_bounds(origin, size)?;
    let (big, minimap_marker, plan, scan) = big_crop(origin, size, bounds);
    let whole = Bounds { x: origin[0], y: origin[1], width: size[0], height: size[1] };
    Ok(Region {
        window,
        bounds,
        big,
        whole,
        minimap_marker,
        plan,
        scan,
    })
}

fn intersects(a: Bounds, b: RECT) -> bool {
    a.x < b.right && a.x + a.width > b.left && a.y < b.bottom && a.y + a.height > b.top
}

struct Occlusion {
    game: HWND,
    crop: Bounds,
    /// Our own windows (the compass sits on the player) don't count: they draw in
    /// colours the big-map reader ignores.
    skip_process: Option<u32>,
    reached_game: bool,
    covered: bool,
}
unsafe extern "system" fn check_window(window: HWND, parameter: LPARAM) -> i32 {
    let check = unsafe { &mut *(parameter as *mut Occlusion) };
    if window == check.game {
        check.reached_game = true;
        return 0;
    }
    if let Some(own) = check.skip_process {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
        }
        if pid == own {
            return 1;
        }
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

impl Region {
    /// Where this app's panel is, for the icon search: only known for a layer surface
    /// (Linux), so the fixed place stays.
    pub fn skip_of(&self, _rect: [f64; 4]) -> [usize; 4] {
        self.scan.skip[0]
    }
}

/// The minimap crop.
pub fn capture(region: &Region) -> Result<Image, String> {
    capture_crop(region, region.bounds, None)
}

/// The big-map crop, read while the minimap is hidden.
pub fn capture_bigmap(region: &Region) -> Result<Image, String> {
    capture_crop(region, region.big, Some(unsafe { GetCurrentProcessId() }))
}

/// The whole client area, read now and then for map icons far from the player.
pub fn capture_window(region: &Region) -> Result<Image, String> {
    capture_crop(region, region.whole, Some(unsafe { GetCurrentProcessId() }))
}

fn capture_crop(region: &Region, bounds: Bounds, skip_process: Option<u32>) -> Result<Image, String> {
    let _dpi = DpiGuard::enter()?;
    if game_window()? != region.window {
        return Err("Game window changed; locating minimap again".into());
    }
    if window_bounds(region.window)? != region.bounds {
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
        skip_process,
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
    if unsafe { GetForegroundWindow() } != region.window
        || window_bounds(region.window)? != region.bounds
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
    fn big_map_crop_surrounds_the_player() {
        // 3840x2160 client at a negative origin: the big map's marker at 1919, 1059.
        let origin = [-3840, 0];
        let minimap = crop_bounds(origin, [3840, 2160]).unwrap();
        let (big, minimap_marker, plan, scan) = big_crop(origin, [3840, 2160], minimap);
        // The whole client: the same marker, panels left out.
        assert!((scan.plan.marker[0] - 1918.8).abs() < 0.1 && (scan.plan.marker[1] - 1059.0).abs() < 0.2);
        assert_eq!(scan.skip[0], [0, 0, 729, 864]);
        let marker = [big.x as f64 + plan.marker[0], big.y as f64 + plan.marker[1]];
        assert!((marker[0] - (-3840.0 + 1918.8)).abs() < 0.1 && (marker[1] - 1059.0).abs() < 0.2, "{marker:?}");
        // Wide frames: 600x518 cells (1200x1036 px) around the player, inside the crop.
        assert_eq!((plan.frame.cols, plan.frame.rows, plan.frame.capture_px), (600, 518, [600, 518]));
        assert!(big.width >= 1200 && big.height >= 1036);
        // The minimap's cross is expected at cell 158, 142 of its own crop.
        assert!((minimap_marker[0] - 316.0).abs() < 0.5 && (minimap_marker[1] - 284.0).abs() < 0.5, "{minimap_marker:?}");
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
