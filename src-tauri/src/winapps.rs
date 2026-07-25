//! Windows-specific application audio plumbing.
//!
//! Two capabilities, both built on documented + battle-tested APIs:
//!
//! 1. **Which apps are playing sound?** — enumerate WASAPI audio sessions on
//!    every active render endpoint (exactly what the Windows volume mixer
//!    shows), resolve each session's process to an .exe name.
//!
//! 2. **Route an app's audio to a device** — the drag-and-drop feature.
//!    Uses the `AudioPolicyConfig` factory (the same non-public interface
//!    EarTrumpet and SoundVolumeView rely on) to set a *persisted* default
//!    render endpoint for a process. Windows itself remembers the mapping
//!    per application, across restarts and reboots.
//!
//! Every entry point creates/uses COM in MTA mode and is safe to call from
//! any Tauri command thread.

// The declared COM interface mirrors the real PascalCase method names.
#![allow(non_snake_case)]

use serde::Serialize;
use windows::core::{interface, IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT, HSTRING};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ, STGM_READWRITE,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::WinRT::RoGetActivationFactory;

/// One application that currently has an audio session (à la volume mixer).
#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
    /// Executable file name, e.g. "Spotify.exe" — the stable identifier.
    pub exe: String,
    /// Display label, e.g. "Spotify".
    pub label: String,
    /// PID of one session owner (informational; may change on restart).
    pub pid: u32,
    /// True if the session is actively rendering right now.
    pub active: bool,
    /// Small icon as a `data:image/bmp;base64,...` URI, when extraction
    /// succeeds. `None` lets the frontend fall back to its glyph picker.
    pub icon: Option<String>,
}

/// Best-effort COM init for the calling thread. MTA; a pre-existing STA on
/// the thread is tolerated (calls still work through implicit marshaling).
fn ensure_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Run `f` on a fresh thread with a clean MTA apartment. Tauri commands can
/// land on the tao main thread (STA, owned by the webview); doing all COM
/// work on our own thread removes every apartment ambiguity.
fn in_com_thread<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    std::thread::Builder::new()
        .name("mixflow-com".into())
        .spawn(move || {
            ensure_com();
            f()
        })
        .map_err(|e| format!("thread COM: {e}"))?
        .join()
        .map_err(|_| "le thread COM a paniqué".to_string())?
}

fn err<T: std::fmt::Display>(context: &str) -> impl Fn(T) -> String + '_ {
    move |e| format!("{context}: {e}")
}

// ---------------------------------------------------------------------------
// 1. Session enumeration
// ---------------------------------------------------------------------------

/// List applications with an audio session on any active render device,
/// deduplicated by executable, active sessions flagged.
pub fn list_apps() -> Result<Vec<AppInfo>, String> {
    let res = in_com_thread(list_apps_inner);
    match &res {
        Ok(apps) => eprintln!("[mixflow] scan apps: {} trouvée(s)", apps.len()),
        Err(e) => eprintln!("[mixflow] scan apps ÉCHEC: {e}"),
    }
    res
}

fn list_apps_inner() -> Result<Vec<AppInfo>, String> {
    let own_pid = std::process::id();
    let mut by_exe: std::collections::HashMap<String, AppInfo> = std::collections::HashMap::new();

    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(err("device enumerator"))?;
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(err("render endpoints"))?;
        for i in 0..devices.GetCount().map_err(err("device count"))? {
            let Ok(device) = devices.Item(i) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
            let count = sessions.GetCount().unwrap_or(0);
            for j in 0..count {
                let Ok(ctl) = sessions.GetSession(j) else {
                    continue;
                };
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let pid = ctl2.GetProcessId().unwrap_or(0);
                if pid == 0 || pid == own_pid {
                    continue; // system sounds session / ourselves
                }
                let Some(path) = process_image_path(pid) else {
                    continue;
                };
                let exe = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
                let active = ctl
                    .GetState()
                    .map(|s| s == AudioSessionStateActive)
                    .unwrap_or(false);
                let label = exe.trim_end_matches(".exe").trim_end_matches(".EXE");
                let entry = by_exe.entry(exe.to_lowercase()).or_insert(AppInfo {
                    exe: exe.clone(),
                    label: capitalize(label),
                    pid,
                    active: false,
                    icon: app_icon_data_uri(&path),
                });
                entry.active |= active;
            }
        }
    }

    // Well-known audio apps that are RUNNING but currently silent (no audio
    // session yet — e.g. Discord idle) still deserve a chip: per-app routing
    // works from any live PID, session or not.
    const KNOWN_APPS: &[&str] = &[
        "discord.exe",
        "spotify.exe",
        "chrome.exe",
        "msedge.exe",
        "firefox.exe",
        "brave.exe",
        "opera.exe",
        "vlc.exe",
        "steam.exe",
        "obs64.exe",
        "deezer.exe",
        "tidal.exe",
    ];
    unsafe {
        if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                    let key = name.to_lowercase();
                    if KNOWN_APPS.contains(&key.as_str()) && !by_exe.contains_key(&key) {
                        let label = name.trim_end_matches(".exe").trim_end_matches(".EXE");
                        let icon = process_image_path(entry.th32ProcessID)
                            .and_then(|p| app_icon_data_uri(&p));
                        by_exe.insert(
                            key,
                            AppInfo {
                                exe: name.clone(),
                                label: capitalize(label),
                                pid: entry.th32ProcessID,
                                active: false,
                                icon,
                            },
                        );
                    }
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
    }

    let mut apps: Vec<AppInfo> = by_exe.into_values().collect();
    // Actively-playing apps first, then alphabetical.
    apps.sort_by(|a, b| b.active.cmp(&a.active).then(a.label.cmp(&b.label)));
    Ok(apps)
}

/// Render endpoint paired with a virtual cable's capture endpoint.
///
/// Robust against MixFlow's own renames: "CABLE-A Input (VB-Audio Cable A)"
/// may have become "Game (VB-Audio Cable A)", but the adapter suffix between
/// parentheses never changes — so we pair capture and render sides by that
/// suffix instead of guessing the name.
pub fn render_id_for_capture(capture_name: &str) -> Result<String, String> {
    let suffix = capture_name
        .rfind('(')
        .map(|i| capture_name[i..].to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("nom de câble inattendu : \"{capture_name}\""))?;
    in_com_thread(move || {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(err("device enumerator"))?;
            let devices = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(err("render endpoints"))?;
            for i in 0..devices.GetCount().map_err(err("device count"))? {
                let Ok(device) = devices.Item(i) else {
                    continue;
                };
                let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                    continue;
                };
                let Ok(name_prop) = store.GetValue(&PKEY_Device_FriendlyName) else {
                    continue;
                };
                if name_prop.to_string().ends_with(&suffix) {
                    let id = device.GetId().map_err(err("device id"))?;
                    return id.to_string().map_err(|e| format!("device id: {e}"));
                }
            }
        }
        Err(format!(
            "côté rendu du câble \"{suffix}\" introuvable — VB-Cable est-il toujours installé ?"
        ))
    })
}

/// L'endpoint de rendu d'id `device_id` existe-t-il encore, et est-il actif ?
///
/// Les ids MMDevice ne survivent PAS à une désinstallation/réinstallation du
/// driver VB-Cable : Windows regénère le GUID d'endpoint. Comme
/// `SetPersistedDefaultAudioEndpoint` accepte volontiers un chemin SWD vers
/// un périphérique absent (Windows persiste sciemment des routes vers du
/// matériel débranché), un `cable_render_id` périmé enverrait les apps vers
/// un endpoint fantôme sans la moindre erreur — d'où cette revalidation
/// avant tout usage du cache. En cas d'échec COM on répond `false` :
/// l'appelant refait alors une résolution par nom, qui est le comportement
/// sûr.
pub fn render_device_active(device_id: &str) -> bool {
    let id = device_id.to_string();
    in_com_thread(move || unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(err("device enumerator"))?;
        let device = enumerator
            .GetDevice(&HSTRING::from(id.as_str()))
            .map_err(err("device lookup"))?;
        let state = device.GetState().map_err(err("device state"))?;
        Ok(state == DEVICE_STATE_ACTIVE)
    })
    .unwrap_or(false)
}

/// Tous les PID à essayer pour router `exe`, les plus prometteurs d'abord.
///
/// Il ne suffit PAS de prendre les PID des sessions audio : les navigateurs
/// Chromium (Brave, Chrome, Edge…) jouent le son depuis un processus
/// utilitaire *sandboxé*, pour lequel Windows ne sait pas résoudre
/// l'identité de l'application — `SetPersistedDefaultAudioEndpoint` y
/// répond `E_INVALIDARG` (0x80070057). Le code s'arrêtait à cette liste dès
/// qu'elle était non vide et n'essayait jamais le processus principal, seul
/// capable d'accepter la route : le routage de Brave échouait donc
/// systématiquement.
///
/// On concatène donc les deux sources en dédoublonnant, et l'appelant tente
/// chaque PID jusqu'à ce que l'un accepte.
fn candidate_pids(exe: &str) -> Vec<u32> {
    let mut pids = pids_for_exe(exe).unwrap_or_default();
    for pid in pids_running_exe(exe) {
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
}

/// PIDs of every audio session currently owned by `exe` (case-insensitive).
fn pids_for_exe(exe: &str) -> Result<Vec<u32>, String> {
    let target = exe.to_lowercase();
    Ok(list_apps_raw_pids()?
        .into_iter()
        .filter(|(e, _)| e.to_lowercase() == target)
        .map(|(_, pid)| pid)
        .collect())
}

/// PIDs of every RUNNING process named `exe` — no audio session required.
/// Fallback so idle apps (Discord before a call…) can still be routed:
/// Windows persists the assignment per application and applies it as soon
/// as the app starts playing.
fn pids_running_exe(exe: &str) -> Vec<u32> {
    let target = exe.to_lowercase();
    let mut out = Vec::new();
    unsafe {
        if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                    if name.to_lowercase() == target {
                        out.push(entry.th32ProcessID);
                    }
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
    }
    out
}

/// (exe, pid) of every audio session — helper for [`pids_for_exe`].
/// Runs inline: callers are already inside a COM thread or command context.
fn list_apps_raw_pids() -> Result<Vec<(String, u32)>, String> {
    ensure_com();
    let mut out = Vec::new();
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(err("device enumerator"))?;
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(err("render endpoints"))?;
        for i in 0..devices.GetCount().map_err(err("device count"))? {
            let Ok(device) = devices.Item(i) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
            for j in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(ctl) = sessions.GetSession(j) else {
                    continue;
                };
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let pid = ctl2.GetProcessId().unwrap_or(0);
                if pid == 0 {
                    continue;
                }
                if let Some(path) = process_image_path(pid) {
                    let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
                    out.push((name, pid));
                }
            }
        }
    }
    Ok(out)
}

fn process_image_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        res.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

// ---------------------------------------------------------------------------
// 1b. App icons (SHGetFileInfo -> data URI) + foreground-window lookup
// ---------------------------------------------------------------------------

fn icon_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, Option<String>>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Extracts the small icon of an executable as a `data:image/bmp;base64,...`
/// URI, cached for the process lifetime — icon extraction is a handful of
/// GDI round-trips, too slow to redo on every ~10 s app-list poll.
fn app_icon_data_uri(exe_path: &str) -> Option<String> {
    if let Some(cached) = icon_cache().lock().ok()?.get(exe_path) {
        return cached.clone();
    }
    let icon = extract_icon_data_uri(exe_path);
    if let Ok(mut cache) = icon_cache().lock() {
        cache.insert(exe_path.to_string(), icon.clone());
    }
    icon
}

fn extract_icon_data_uri(exe_path: &str) -> Option<String> {
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    unsafe {
        let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut shfi = SHFILEINFOW::default();
        let res = SHGetFileInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        );
        if res == 0 || shfi.hIcon.is_invalid() {
            return None;
        }
        let hicon = shfi.hIcon;

        let mut info = ICONINFO::default();
        if GetIconInfo(hicon, &mut info).is_err() {
            let _ = DestroyIcon(hicon);
            return None;
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask.into());
        }
        if info.hbmColor.is_invalid() {
            let _ = DestroyIcon(hicon);
            return None;
        }

        let mut bmp = BITMAP::default();
        let got_obj = GetObjectW(
            info.hbmColor.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut BITMAP as *mut core::ffi::c_void),
        );
        if got_obj == 0 {
            let _ = DeleteObject(info.hbmColor.into());
            let _ = DestroyIcon(hicon);
            return None;
        }
        let (w, h) = (bmp.bmWidth, bmp.bmHeight);
        if w <= 0 || h <= 0 || w > 256 || h > 256 {
            let _ = DeleteObject(info.hbmColor.into());
            let _ = DestroyIcon(hicon);
            return None;
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: h, // positive = bottom-up, matches GetDIBits' default output
                biPlanes: 1,
                biBitCount: 32,
                biCompression: DIB_RGB_COLORS.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let row_bytes = (w as usize) * 4;
        let mut pixels = vec![0u8; row_bytes * h as usize];
        let dc = GetDC(None);
        let got = GetDIBits(
            dc,
            info.hbmColor,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DestroyIcon(hicon);
        if got == 0 {
            return None;
        }

        // Minimal on-disk BMP: 14-byte file header + 40-byte info header +
        // the bottom-up BGRA pixel data GetDIBits just filled in.
        let pixel_offset: u32 = 14 + 40;
        let mut file = Vec::with_capacity(pixel_offset as usize + pixels.len());
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&(pixel_offset + pixels.len() as u32).to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes()); // reserved
        file.extend_from_slice(&pixel_offset.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER size
        file.extend_from_slice(&w.to_le_bytes());
        file.extend_from_slice(&h.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes()); // planes
        file.extend_from_slice(&32u16.to_le_bytes()); // bpp
        file.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        file.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        file.extend_from_slice(&0i32.to_le_bytes());
        file.extend_from_slice(&0i32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&pixels);

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&file);
        Some(format!("data:image/bmp;base64,{b64}"))
    }
}

/// Executable name of whatever window currently has focus — used to
/// auto-switch mix profiles (see `main.rs`'s profile-watch thread).
pub fn foreground_exe() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        process_image_path(pid).map(|p| p.rsplit(['\\', '/']).next().unwrap_or(&p).to_string())
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// 2. Per-app output routing (AudioPolicyConfig)
// ---------------------------------------------------------------------------

/// The Windows 11 / Windows 10 21H2+ AudioPolicyConfig factory. Method order
/// mirrors EarTrumpet's definition — only the last three slots are real, the
/// leading ones are vtable padding we never call. The real interface derives
/// from IInspectable; we declare it on IUnknown and pad the 3 IInspectable
/// slots (GetIids / GetRuntimeClassName / GetTrustLevel) by hand.
#[interface("ab3d4648-e242-459f-b02f-541c70306324")]
unsafe trait IAudioPolicyConfigFactory: IUnknown {
    fn insp_get_iids(&self) -> HRESULT;
    fn insp_get_runtime_class_name(&self) -> HRESULT;
    fn insp_get_trust_level(&self) -> HRESULT;
    fn pad01(&self) -> HRESULT;
    fn pad02(&self) -> HRESULT;
    fn pad03(&self) -> HRESULT;
    fn pad04(&self) -> HRESULT;
    fn pad05(&self) -> HRESULT;
    fn pad06(&self) -> HRESULT;
    fn pad07(&self) -> HRESULT;
    fn pad08(&self) -> HRESULT;
    fn pad09(&self) -> HRESULT;
    fn pad10(&self) -> HRESULT;
    fn pad11(&self) -> HRESULT;
    fn pad12(&self) -> HRESULT;
    fn pad13(&self) -> HRESULT;
    fn pad14(&self) -> HRESULT;
    fn pad15(&self) -> HRESULT;
    fn pad16(&self) -> HRESULT;
    fn pad17(&self) -> HRESULT;
    fn pad18(&self) -> HRESULT;
    fn pad19(&self) -> HRESULT;
    fn SetPersistedDefaultAudioEndpoint(
        &self,
        process_id: u32,
        flow: i32,
        role: i32,
        device_id: *mut core::ffi::c_void,
    ) -> HRESULT;
    fn GetPersistedDefaultAudioEndpoint(
        &self,
        process_id: u32,
        flow: i32,
        role: i32,
        device_id: *mut *mut core::ffi::c_void,
    ) -> HRESULT;
    fn ClearAllPersistedApplicationDefaultEndpoints(&self) -> HRESULT;
}

const E_RENDER: i32 = 0; // EDataFlow::eRender
const ROLE_CONSOLE: i32 = 0; // ERole::eConsole
const ROLE_MULTIMEDIA: i32 = 1; // ERole::eMultimedia

fn policy_factory() -> Result<IAudioPolicyConfigFactory, String> {
    ensure_com();
    unsafe {
        RoGetActivationFactory::<IAudioPolicyConfigFactory>(&HSTRING::from(
            "Windows.Media.Internal.AudioPolicyConfig",
        ))
        .map_err(|e| format!("AudioPolicyConfig indisponible sur cette version de Windows: {e}"))
    }
}

/// PKEY_Device_DeviceDesc — the editable part of an endpoint's display name
/// ("CABLE Input" in "CABLE Input (VB-Audio Virtual Cable)"). This is the
/// property the Windows sound control panel writes when a user renames a
/// device.
const PKEY_DEVICE_DESC: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 2,
};

/// Rename a render endpoint (by MMDevice id) so it shows up as e.g.
/// "Game (VB-Audio Virtual Cable)" in every Windows output-device picker —
/// the Sonar experience.
pub fn rename_render_device(device_id: &str, new_desc: &str) -> Result<(), String> {
    // Un nom vide laisserait le périphérique SANS étiquette dans tout
    // Windows — refuser plutôt que d'abîmer la config son de l'utilisateur
    // (les appelants dérivent parfois ce nom d'une ligne introuvable).
    if new_desc.trim().is_empty() {
        return Err("nom vide : renommage ignoré".into());
    }
    let (id, desc) = (device_id.to_string(), new_desc.to_string());
    in_com_thread(move || rename_render_device_inner(&id, &desc))
}

fn rename_render_device_inner(device_id: &str, new_desc: &str) -> Result<(), String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(err("device enumerator"))?;
        let device = enumerator
            .GetDevice(&HSTRING::from(device_id))
            .map_err(err("device lookup"))?;
        let store = device.OpenPropertyStore(STGM_READWRITE).map_err(|e| {
            format!(
                "renommage refusé ({e}) — lance MixFlow en administrateur une fois, ou renomme le périphérique dans Paramètres → Son"
            )
        })?;
        // ATTENTION au type de PROPVARIANT. `PROPVARIANT::from(&str)` de
        // windows-rs construit un **VT_BSTR** (il passe par `BSTR::from`),
        // que le magasin de propriétés sérialise en REG_BINARY. Or Windows
        // stocke `PKEY_Device_DeviceDesc` en **VT_LPWSTR** (REG_SZ) : avec un
        // BSTR, la valeur est bien écrite mais le panneau Son et les
        // sélecteurs de périphériques n'arrivent pas à la lire et affichent
        // un nom VIDE. On convertit donc explicitement en VT_LPWSTR.
        use windows::Win32::System::Com::StructuredStorage::{PropVariantChangeType, PROPVARIANT};
        use windows::Win32::System::Variant::VT_LPWSTR;
        let as_bstr = PROPVARIANT::from(new_desc);
        let mut value = PROPVARIANT::default();
        PropVariantChangeType(
            &mut value,
            &as_bstr,
            windows::Win32::System::Com::StructuredStorage::PROPVAR_CHANGE_FLAGS(0),
            VT_LPWSTR,
        )
        .map_err(err("conversion du nom en VT_LPWSTR"))?;
        store
            .SetValue(&PKEY_DEVICE_DESC, &value)
            .map_err(|e| format!("renommage refusé ({e}) — essaie en administrateur"))?;
        let _ = store.Commit();
    }
    Ok(())
}

/// Wrap an MMDevice id in the SWD path format the policy API expects.
fn policy_device_path(mmdevice_id: &str) -> HSTRING {
    // {e6327cad-...} = DEVINTERFACE_AUDIO_RENDER
    HSTRING::from(format!(
        r"\\?\SWD#MMDEVAPI#{mmdevice_id}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}"
    ))
}

/// HSTRING is a repr(transparent) handle; the raw pointer is what crosses
/// the vtable. Null = "reset to the system default".
fn hstring_raw(h: &HSTRING) -> *mut core::ffi::c_void {
    unsafe { std::mem::transmute_copy(h) }
}

/// Route every current audio session of `exe` to the render device with this
/// MMDevice id. Windows persists the mapping per application.
pub fn route_app_by_id(exe: &str, device_id: &str) -> Result<(), String> {
    let (exe, id) = (exe.to_string(), device_id.to_string());
    in_com_thread(move || route_app_by_id_inner(&exe, &id))
}

fn route_app_by_id_inner(exe: &str, device_id: &str) -> Result<(), String> {
    let pids = candidate_pids(exe);
    if pids.is_empty() {
        return Err(format!(
            "\"{exe}\" ne semble pas lancée — démarre l'application puis réessaie"
        ));
    }
    let path = policy_device_path(device_id);
    let factory = policy_factory()?;
    // Multi-process apps: it only takes one PID que Windows sait rattacher à
    // l'identité de l'app — on tolère les échecs unitaires (les processus
    // sandboxés des navigateurs répondent E_INVALIDARG) et on n'échoue que
    // si AUCUN n'a accepté.
    let mut successes = 0;
    let mut last_err = String::new();
    unsafe {
        for pid in &pids {
            for role in [ROLE_CONSOLE, ROLE_MULTIMEDIA] {
                match factory
                    .SetPersistedDefaultAudioEndpoint(*pid, E_RENDER, role, hstring_raw(&path))
                    .ok()
                {
                    Ok(()) => successes += 1,
                    Err(e) => last_err = e.to_string(),
                }
            }
        }
    }
    if successes == 0 {
        return Err(format!(
            "routage de {exe} refusé après {} processus essayé(s) : {last_err} — \
             si l'app vient d'être lancée, fais-lui jouer un son puis réessaie",
            pids.len()
        ));
    }
    Ok(())
}

/// Reset `exe` to the system default output. Needs a live session to name a
/// PID; if the app isn't running this is a no-op (the persisted entry will
/// be overwritten next time it's routed anyway).
pub fn unroute_app(exe: &str) -> Result<(), String> {
    let exe = exe.to_string();
    in_com_thread(move || unroute_app_inner(&exe))
}

fn unroute_app_inner(exe: &str) -> Result<(), String> {
    // Mêmes candidats qu'au routage : ne défaire que sur les PID de session
    // laisserait la route persistée sur le processus principal.
    let pids = candidate_pids(exe);
    if pids.is_empty() {
        return Ok(());
    }
    let factory = policy_factory()?;
    unsafe {
        for pid in pids {
            for role in [ROLE_CONSOLE, ROLE_MULTIMEDIA] {
                let _ = factory
                    .SetPersistedDefaultAudioEndpoint(pid, E_RENDER, role, std::ptr::null_mut())
                    .ok();
            }
        }
    }
    Ok(())
}
