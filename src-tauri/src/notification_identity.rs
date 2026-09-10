use serde::Serialize;

pub const APP_USER_MODEL_ID: &str = "com.chinesecanfly-wxy.bull-arrives";

#[derive(Debug, Clone, Serialize)]
pub struct NotificationIdentityStatus {
    pub supported: bool,
    pub registered: bool,
    pub shortcut_path: Option<String>,
    pub detail: String,
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{NotificationIdentityStatus, APP_USER_MODEL_ID};
    use std::{fs, path::{Path, PathBuf}};
    use windows::{
        core::{Interface, PCWSTR},
        Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED},
        Win32::UI::Shell::{IShellLinkW, ShellLink},
        Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY},
    };
    use windows::core::PROPVARIANT;
    const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

    struct ComGuard;
    impl Drop for ComGuard { fn drop(&mut self) { unsafe { CoUninitialize() } } }

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        value.encode_wide().chain(Some(0)).collect()
    }

    fn shortcut_path() -> Result<PathBuf, String> {
        let appdata = std::env::var_os("APPDATA").ok_or("无法读取当前用户 APPDATA")?;
        Ok(PathBuf::from(appdata).join("Microsoft/Windows/Start Menu/Programs/Bull Arrives.lnk"))
    }

    unsafe fn initialize_com() -> Result<ComGuard, String> {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(|e| format!("初始化 Windows Shell COM 失败: {e}"))?;
        Ok(ComGuard)
    }

    unsafe fn load_link(path: &Path) -> Result<(IShellLinkW, IPropertyStore), String> {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("创建 ShellLink COM 对象失败: {e}"))?;
        let persist: IPersistFile = link.cast().map_err(|e| format!("读取快捷方式接口失败: {e}"))?;
        persist.Load(PCWSTR(wide(path.as_os_str()).as_ptr()), windows::Win32::System::Com::STGM(0))
            .map_err(|e| format!("读取现有快捷方式失败: {e}"))?;
        let store: IPropertyStore = link.cast().map_err(|e| format!("读取快捷方式属性失败: {e}"))?;
        Ok((link, store))
    }

    unsafe fn existing_owned(path: &Path, exe: &Path) -> Result<bool, String> {
        if !path.exists() { return Ok(false); }
        let (link, store) = load_link(path)?;
        let mut target = [0u16; 32768];
        link.GetPath(&mut target, std::ptr::null_mut(), 0)
            .map_err(|e| format!("读取快捷方式目标失败: {e}"))?;
        let target_len = target.iter().position(|c| *c == 0).unwrap_or(target.len());
        let target = PathBuf::from(String::from_utf16_lossy(&target[..target_len]));
        let value = store.GetValue(&PKEY_APP_USER_MODEL_ID)
            .map_err(|e| format!("读取快捷方式 AUMID 失败: {e}"))?;
        let aumid = value.to_string();
        Ok(paths_equal(&target, exe) && aumid == APP_USER_MODEL_ID)
    }

    fn paths_equal(left: &Path, right: &Path) -> bool {
        let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
        let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
        left.to_string_lossy().eq_ignore_ascii_case(&right.to_string_lossy())
    }

    pub fn status() -> Result<NotificationIdentityStatus, String> {
        let path = shortcut_path()?;
        let exe = std::env::current_exe().map_err(|e| format!("读取程序路径失败: {e}"))?;
        let registered = unsafe {
            let _com = initialize_com()?;
            existing_owned(&path, &exe).unwrap_or(false)
        };
        Ok(NotificationIdentityStatus {
            supported: true,
            registered,
            shortcut_path: Some(path.to_string_lossy().into_owned()),
            detail: if registered { "通知身份已注册".into() } else { "尚未注册当前程序的通知身份".into() },
        })
    }

    pub fn register() -> Result<NotificationIdentityStatus, String> {
        let path = shortcut_path()?;
        let exe = std::env::current_exe().map_err(|e| format!("读取程序路径失败: {e}"))?;
        unsafe {
            let _com = initialize_com()?;
            if path.exists() && !existing_owned(&path, &exe)? {
                return Err(format!("开始菜单中已存在非本程序拥有的同名快捷方式，未覆盖: {}", path.display()));
            }
            if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| format!("创建开始菜单目录失败: {e}"))?; }
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("创建 ShellLink COM 对象失败: {e}"))?;
            let exe_w = wide(exe.as_os_str());
            link.SetPath(PCWSTR(exe_w.as_ptr())).map_err(|e| format!("设置快捷方式目标失败: {e}"))?;
            if let Some(parent) = exe.parent() {
                let parent_w = wide(parent.as_os_str());
                link.SetWorkingDirectory(PCWSTR(parent_w.as_ptr())).map_err(|e| format!("设置工作目录失败: {e}"))?;
            }
            link.SetIconLocation(PCWSTR(exe_w.as_ptr()), 0).map_err(|e| format!("设置图标失败: {e}"))?;
            let store: IPropertyStore = link.cast().map_err(|e| format!("打开快捷方式属性失败: {e}"))?;
            let value = PROPVARIANT::from(APP_USER_MODEL_ID);
            store.SetValue(&PKEY_APP_USER_MODEL_ID, &value).map_err(|e| format!("写入 AUMID 失败: {e}"))?;
            store.Commit().map_err(|e| format!("保存 AUMID 失败: {e}"))?;
            let persist: IPersistFile = link.cast().map_err(|e| format!("打开快捷方式保存接口失败: {e}"))?;
            persist.Save(PCWSTR(wide(path.as_os_str()).as_ptr()), true)
                .map_err(|e| format!("保存开始菜单快捷方式失败: {e}"))?;
        }
        status()
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::NotificationIdentityStatus;
    pub fn status() -> Result<NotificationIdentityStatus, String> { Ok(NotificationIdentityStatus { supported: false, registered: false, shortcut_path: None, detail: "仅 Windows 支持通知身份注册".into() }) }
    pub fn register() -> Result<NotificationIdentityStatus, String> { Err("仅 Windows 支持通知身份注册".into()) }
}

#[tauri::command]
pub fn get_notification_identity_status() -> Result<NotificationIdentityStatus, String> { platform::status() }

/// This mutating command is never called at startup; registration occurs only after an explicit UI click.
#[tauri::command]
pub fn register_notification_identity() -> Result<NotificationIdentityStatus, String> { platform::register() }
