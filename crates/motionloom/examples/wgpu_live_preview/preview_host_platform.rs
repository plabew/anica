#[cfg(target_os = "macos")]
pub fn frontmost_process_id() -> Option<u32> {
    use objc2_app_kit::NSWorkspace;

    let workspace = NSWorkspace::sharedWorkspace();
    let application = workspace.frontmostApplication()?;
    let pid = application.processIdentifier();
    (pid > 0).then_some(pid as u32)
}

#[cfg(target_os = "windows")]
pub fn frontmost_process_id() -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid > 0).then_some(pid)
    }
}

#[cfg(target_os = "linux")]
pub fn frontmost_process_id() -> Option<u32> {
    if std::env::var_os("DISPLAY").is_none() {
        return None;
    }

    frontmost_process_id_x11()
}

#[cfg(target_os = "linux")]
fn frontmost_process_id_x11() -> Option<u32> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (connection, screen_num) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots.get(screen_num)?.root;

    let active_window_atom = connection
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let pid_atom = connection
        .intern_atom(false, b"_NET_WM_PID")
        .ok()?
        .reply()
        .ok()?
        .atom;

    let active_window = connection
        .get_property(false, root, active_window_atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()?;

    connection
        .get_property(false, active_window, pid_atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn frontmost_process_id() -> Option<u32> {
    None
}
