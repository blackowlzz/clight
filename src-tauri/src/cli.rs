#[cfg(windows)]
pub use windows::{handle_flag, install};

fn is_entry(entry: &str, dir: &str) -> bool {
    entry
        .trim()
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(dir)
}

fn with_entry(path: &str, dir: &str) -> Option<String> {
    if path.split(';').any(|e| is_entry(e, dir)) {
        return None;
    }
    let path = path.trim_end_matches(';');
    Some(if path.is_empty() {
        dir.to_string()
    } else {
        format!("{path};{dir}")
    })
}

fn without_entry(path: &str, dir: &str) -> Option<String> {
    if !path.split(';').any(|e| is_entry(e, dir)) {
        return None;
    }
    let kept: Vec<&str> = path.split(';').filter(|e| !is_entry(e, dir)).collect();
    Some(kept.join(";"))
}

#[cfg(windows)]
mod windows {
    use super::{with_entry, without_entry};
    use std::borrow::Cow;
    use std::fs;
    use std::io::ErrorKind;
    use std::path::PathBuf;
    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::{RegKey, RegValue};

    // .cmd rather than .exe: CMake's compiler search only looks at .exe and .com,
    // so a shim called cl can't be mistaken for the MSVC compiler.
    const NAMES: [&str; 2] = ["clight", "cl"];

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    fn exe() -> Result<PathBuf, String> {
        std::env::current_exe().map_err(|e| e.to_string())
    }

    fn bin_dir() -> Result<PathBuf, String> {
        let exe = exe()?;
        let parent = exe.parent().ok_or("The install folder has no parent")?;
        Ok(parent.join("bin"))
    }

    fn broadcast_environment_changed() {
        let name: Vec<u16> = "Environment\0".encode_utf16().collect();
        let mut result = 0usize;
        unsafe {
            SendMessageTimeoutW(0xffff, 0x001a, 0, name.as_ptr(), 2, 2000, &mut result);
        }
    }

    // Goes through the raw value so %VARIABLES% in the user's Path stay unexpanded.
    fn edit_path(edit: impl Fn(&str) -> Option<String>) -> Result<(), String> {
        let env = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", KEY_READ | KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        let (current, vtype) = match env.get_raw_value("Path") {
            Ok(v) => {
                let wide: Vec<u16> = v
                    .bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let text = String::from_utf16(&wide).map_err(|e| e.to_string())?;
                (text.trim_end_matches('\0').to_string(), v.vtype)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => (String::new(), RegType::REG_EXPAND_SZ),
            Err(e) => return Err(e.to_string()),
        };
        if !matches!(vtype, RegType::REG_SZ | RegType::REG_EXPAND_SZ) {
            return Err("The user Path has an unexpected type".into());
        }
        let Some(next) = edit(&current) else {
            return Ok(());
        };
        let bytes: Vec<u8> = next
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        env.set_raw_value(
            "Path",
            &RegValue {
                bytes: Cow::Owned(bytes),
                vtype,
            },
        )
        .map_err(|e| e.to_string())?;
        broadcast_environment_changed();
        Ok(())
    }

    pub fn install() -> Result<String, String> {
        let exe = exe()?;
        let exe_name = exe
            .file_name()
            .ok_or("The program has no name")?
            .to_string_lossy()
            .into_owned();
        let dir = bin_dir()?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Relative to the shim, because .cmd files can't hold non-ASCII paths reliably.
        let shim = format!("@start \"\" \"%~dp0..\\{exe_name}\" %*\r\n");
        for name in NAMES {
            fs::write(dir.join(format!("{name}.cmd")), &shim).map_err(|e| e.to_string())?;
        }
        let dir = dir.to_string_lossy().into_owned();
        edit_path(|p| with_entry(p, &dir))?;
        Ok(NAMES.join(" and "))
    }

    fn remove() -> Result<(), String> {
        let dir = bin_dir()?;
        for name in NAMES {
            let _ = fs::remove_file(dir.join(format!("{name}.cmd")));
        }
        let _ = fs::remove_dir(&dir);
        let dir = dir.to_string_lossy().into_owned();
        edit_path(|p| without_entry(p, &dir))
    }

    /// Runs the installer's `--install-cli` and `--remove-cli` hooks.
    pub fn handle_flag() -> Option<i32> {
        let result = match std::env::args().nth(1).as_deref() {
            Some("--install-cli") => install().map(|_| ()),
            Some("--remove-cli") => remove(),
            _ => return None,
        };
        Some(i32::from(result.is_err()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = r"C:\Users\me\AppData\Local\clight\bin";

    #[test]
    fn the_entry_is_appended_once() {
        let next = with_entry(r"%USERPROFILE%\go\bin;C:\tools;", DIR).unwrap();
        assert_eq!(next, format!(r"%USERPROFILE%\go\bin;C:\tools;{DIR}"));
        assert_eq!(with_entry(&next, DIR), None);
        assert_eq!(with_entry(&format!(r"{}\", DIR.to_uppercase()), DIR), None);
        assert_eq!(with_entry("", DIR).as_deref(), Some(DIR));
    }

    #[test]
    fn removing_keeps_everything_else() {
        let path = format!(r"C:\a;{DIR};%X%\b");
        assert_eq!(without_entry(&path, DIR).as_deref(), Some(r"C:\a;%X%\b"));
        assert_eq!(without_entry(r"C:\a", DIR), None);
    }
}
