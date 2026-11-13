//! Game presence from OS process metadata only; no game memory or input.
fn is_game(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(),
        "pathofexile.exe" | "pathofexilesteam.exe" | "pathofexile_x64.exe" | "pathofexile_x64steam.exe")
}

#[cfg(target_os = "windows")]
pub fn running() -> Option<bool> {
    use windows_sys::Win32::{Foundation::{CloseHandle, INVALID_HANDLE_VALUE}, UI::WindowsAndMessaging::EnumWindows, System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    }};
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE { return None; }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry);
        if more == 0 { CloseHandle(snapshot); return None; }
        let mut candidates = GameWindows { pids: Vec::new(), found: false };
        while more != 0 {
            let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if is_game(&String::from_utf16_lossy(&entry.szExeFile[..end])) { candidates.pids.push(entry.th32ProcessID); }
            more = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        // PoE 1 shares the executable names. Confirm the PoE 2 window title,
        // including minimized/background windows, without requiring focus.
        if !candidates.pids.is_empty() {
            EnumWindows(Some(find_game_window), &mut candidates as *mut _ as isize);
        }
        Some(candidates.found)
    }
}

#[cfg(target_os = "windows")]
struct GameWindows { pids: Vec<u32>, found: bool }

#[cfg(target_os = "windows")]
unsafe extern "system" fn find_game_window(window: windows_sys::Win32::Foundation::HWND, param: isize) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextW, GetWindowThreadProcessId};
    let state = &mut *(param as *mut GameWindows);
    let mut pid = 0;
    GetWindowThreadProcessId(window, &mut pid);
    if state.pids.contains(&pid) {
        let mut title = [0u16; 256];
        let len = GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32);
        if len > 0 && String::from_utf16_lossy(&title[..len as usize]) == "Path of Exile 2" {
            state.found = true;
            return 0;
        }
    }
    1
}

#[cfg(target_os = "linux")]
pub fn running() -> Option<bool> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        if !entry.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) { continue; }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        // Wine/Proton's executable can be wine-preloader; the command line names the client.
        if cmdline.split(|&b| b == 0).any(|arg| {
            let arg = String::from_utf8_lossy(arg);
            is_game(arg.rsplit(['/', '\\']).next().unwrap_or(""))
        }) { return Some(true); }
    }
    Some(false)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn running() -> Option<bool> { None }
