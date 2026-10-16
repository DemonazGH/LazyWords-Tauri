use std::{io, path::Path, ptr};

type HKey = isize;
const HKEY_USERS: HKey = 0x80000003u32 as i32 as HKey;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_MORE_DATA: i32 = 234;
const REG_SZ: u32 = 1;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        root: HKey,
        key: *const u16,
        options: u32,
        access: u32,
        result: *mut HKey,
    ) -> i32;
    fn RegQueryValueExW(
        key: HKey,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegDeleteValueW(key: HKey, name: *const u16) -> i32;
    fn RegCloseKey(key: HKey) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CompareStringOrdinal(
        left: *const u16,
        left_len: i32,
        right: *const u16,
        right_len: i32,
        ignore_case: i32,
    ) -> i32;
}

#[link(name = "msi")]
extern "system" {
    fn MsiGetPropertyW(install: u32, name: *const u16, value: *mut u16, size: *mut u32) -> u32;
    fn MsiCreateRecord(fields: u32) -> u32;
    fn MsiRecordSetStringW(record: u32, field: u32, value: *const u16) -> u32;
    fn MsiProcessMessage(install: u32, kind: u32, record: u32) -> i32;
    fn MsiCloseHandle(handle: u32) -> u32;
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct RegistryKey(HKey);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn paths_equal(left: &str, right: &str) -> bool {
    let left: Vec<u16> = left.replace('/', "\\").encode_utf16().collect();
    let right: Vec<u16> = right.replace('/', "\\").encode_utf16().collect();
    unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        ) == 2
    }
}

fn command_targets_executable(command: &str, executable: &str) -> bool {
    let command = command.trim_matches([' ', '\t']);
    if let Some(quoted) = command.strip_prefix('"') {
        return quoted.find('"').is_some_and(|end| {
            paths_equal(&quoted[..end], executable)
                && quoted[end + 1..]
                    .chars()
                    .next()
                    .is_none_or(|c| c == ' ' || c == '\t')
        });
    }

    // Unquoted paths from auto-launch may contain spaces. Match the entire known
    // EXE path, not the first command-line token or a directory prefix.
    let path_chars = executable.chars().count();
    let boundary = command
        .char_indices()
        .nth(path_chars)
        .map(|(i, _)| i)
        .unwrap_or(command.len());
    paths_equal(&command[..boundary], executable)
        && command[boundary..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

fn read_string(key: HKey, name: &[u16]) -> io::Result<Option<String>> {
    for _ in 0..3 {
        let mut kind = 0;
        let mut size = 0;
        let status = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 && status != ERROR_MORE_DATA {
            return Err(io::Error::from_raw_os_error(status));
        }
        if kind != REG_SZ || size > 131_072 {
            return Ok(None);
        }
        let mut bytes = vec![0u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                bytes.as_mut_ptr(),
                &mut size,
            )
        };
        if status == ERROR_MORE_DATA {
            continue;
        }
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if kind != REG_SZ || size % 2 != 0 {
            return Ok(None);
        }
        let mut units: Vec<u16> = bytes[..size as usize]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if units.last() == Some(&0) {
            units.pop();
        }
        return Ok(String::from_utf16(&units)
            .ok()
            .filter(|s| !s.contains('\0')));
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "Run value kept changing",
    ))
}

fn cleanup_value(root: HKey, key_path: &str, name: &str, executable: &str) -> io::Result<bool> {
    let mut handle = 0;
    let status = unsafe {
        RegOpenKeyExW(
            root,
            wide(key_path).as_ptr(),
            0,
            0x0001 | 0x0002,
            &mut handle,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    let key = RegistryKey(handle);
    let name = wide(name);
    let Some(command) = read_string(key.0, &name)? else {
        return Ok(false);
    };
    if !command_targets_executable(&command, executable) {
        return Ok(false);
    }
    // Recheck immediately before deletion in case another installation replaced it.
    if read_string(key.0, &name)?.as_deref() != Some(command.as_str()) {
        return Ok(false);
    }
    let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
    if status != 0 && status != ERROR_FILE_NOT_FOUND {
        return Err(io::Error::from_raw_os_error(status));
    }
    Ok(status == 0)
}

fn parse_action_data(data: &str) -> io::Result<(&str, &str, &str)> {
    let parts: Vec<&str> = data.split('|').collect();
    if parts.len() != 3
        || !parts[0].starts_with("S-1-")
        || parts[0].len() > 184
        || !parts[0][2..]
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-')
        || !Path::new(parts[1]).is_absolute()
        || !parts[1].to_ascii_lowercase().ends_with(".exe")
        || parts[2].is_empty()
        || data.contains('\0')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid cleanup action data",
        ));
    }
    Ok((parts[0], parts[1], parts[2]))
}

fn read_property(mut get: impl FnMut(&mut [u16], &mut u32) -> u32) -> io::Result<String> {
    let mut size = 0;
    let mut empty = [0];
    let status = get(&mut empty, &mut size);
    if status != 0 && status != ERROR_MORE_DATA as u32 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    if size > 65_536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Cleanup action data too long",
        ));
    }
    let mut buffer = vec![0u16; size as usize + 1];
    // MSI returns the text length, but input capacity must include the terminator.
    size = buffer.len() as u32;
    let status = get(&mut buffer, &mut size);
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    if size as usize >= buffer.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid property length",
        ));
    }
    String::from_utf16(&buffer[..size as usize])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-16 action data"))
}

fn action_data(install: u32) -> io::Result<String> {
    let property = wide("CustomActionData");
    read_property(|buffer, size| unsafe {
        MsiGetPropertyW(install, property.as_ptr(), buffer.as_mut_ptr(), size)
    })
}

fn log_message(install: u32, message: &str) {
    unsafe {
        let record = MsiCreateRecord(0);
        if record != 0 {
            MsiRecordSetStringW(record, 0, wide(message).as_ptr());
            MsiProcessMessage(install, 0x04000000, record);
            MsiCloseHandle(record);
        }
    }
}

#[no_mangle]
pub extern "system" fn CleanupOwnedAutostart(install: u32) -> u32 {
    let result = std::panic::catch_unwind(|| -> io::Result<bool> {
        let data = action_data(install)?;
        let (sid, executable, name) = parse_action_data(&data)?;
        // Explicit UserSID avoids using the MSI service's HKCU hive.
        cleanup_value(HKEY_USERS, &format!("{sid}\\{RUN_KEY}"), name, executable)
    });
    match result {
        Ok(Ok(removed)) => {
            log_message(
                install,
                if removed {
                    "LazyWords: owned autostart removed"
                } else {
                    "LazyWords: autostart absent or owned by another installation"
                },
            );
            0
        }
        Ok(Err(error)) => {
            log_message(
                install,
                &format!("LazyWords: autostart cleanup failed: {error}"),
            );
            1603
        }
        Err(_) => 1603,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const MSI_EXE: &str = r"C:\Program Files\LazyWords-QA-MSI\lazywords.exe";
    const NSIS_EXE: &str = r"C:\Users\Example\AppData\Local\LazyWords\lazywords.exe";
    const HKCU: HKey = 0x80000001u32 as i32 as HKey;
    static NEXT: AtomicU32 = AtomicU32::new(0);

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            root: HKey,
            key: *const u16,
            reserved: u32,
            class: *mut u16,
            options: u32,
            access: u32,
            security: *mut std::ffi::c_void,
            result: *mut HKey,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: HKey,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            size: u32,
        ) -> i32;
        fn RegDeleteKeyW(root: HKey, key: *const u16) -> i32;
    }

    struct Fixture {
        path: String,
        key: RegistryKey,
    }

    impl Fixture {
        fn new() -> Self {
            let path = format!(
                r"Software\LazyWords-QA\AutostartCleanup\{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let mut handle = 0;
            assert_eq!(
                unsafe {
                    RegCreateKeyExW(
                        HKCU,
                        wide(&path).as_ptr(),
                        0,
                        ptr::null_mut(),
                        0,
                        0x0001 | 0x0002,
                        ptr::null_mut(),
                        &mut handle,
                        ptr::null_mut(),
                    )
                },
                0
            );
            Self {
                path,
                key: RegistryKey(handle),
            }
        }

        fn set(&self, name: &str, command: &str) {
            let data = wide(command);
            assert_eq!(
                unsafe {
                    RegSetValueExW(
                        self.key.0,
                        wide(name).as_ptr(),
                        0,
                        REG_SZ,
                        data.as_ptr() as *const u8,
                        (data.len() * 2) as u32,
                    )
                },
                0
            );
        }

        fn cleanup(&self) -> bool {
            cleanup_value(HKCU, &self.path, "LazyWords", MSI_EXE).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                RegDeleteKeyW(HKCU, wide(&self.path).as_ptr());
            }
        }
    }

    #[test]
    fn matches_quoted_unquoted_whitespace_case_and_arguments() {
        for value in [
            format!("{MSI_EXE} "),
            format!("\"{MSI_EXE}\""),
            format!(" \"{MSI_EXE}\" --silent"),
            format!("{MSI_EXE}\t--silent"),
            MSI_EXE.to_ascii_uppercase(),
            MSI_EXE.replace('\\', "/"),
        ] {
            assert!(command_targets_executable(&value, MSI_EXE), "{value}");
        }
    }

    #[test]
    fn preserves_other_paths_prefixes_wrappers_and_malformed_quotes() {
        for value in [
            NSIS_EXE.to_string(),
            format!("{MSI_EXE}.bak"),
            format!("\"{MSI_EXE}\"suffix"),
            format!("\"{MSI_EXE}"),
            format!("cmd /c {MSI_EXE}"),
            String::new(),
        ] {
            assert!(!command_targets_executable(&value, MSI_EXE), "{value}");
        }
    }

    #[test]
    fn compares_unicode_windows_paths_without_ascii_case_folding() {
        assert!(command_targets_executable(
            "C:\\Users\\\u{00e4}\\lazywords.exe",
            "C:\\Users\\\u{00c4}\\lazywords.exe"
        ));
    }

    #[test]
    fn validates_sid_and_absolute_executable_action_data() {
        let data = format!("S-1-5-21-1001|{MSI_EXE}|LazyWords");
        assert!(parse_action_data(&data).is_ok());
        for data in [
            "",
            "S-1-5-21\\Other|C:\\a.exe|LazyWords",
            "S-1-5-21|a.exe|LazyWords",
            "S-1-5-21|C:\\a.exe|",
            "S-1-5-21|C:\\a.exe|LazyWords|extra",
        ] {
            assert!(parse_action_data(data).is_err());
        }
    }

    #[test]
    fn reads_msi_property_with_capacity_including_null_terminator() {
        let text = format!("S-1-5-21-1001|C:\\Users\\\u{00e4}\\lazywords.exe|LazyWords");
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut calls = 0;
        let result = read_property(|buffer, size| {
            calls += 1;
            let capacity = *size as usize;
            *size = units.len() as u32;
            if capacity <= units.len() {
                return ERROR_MORE_DATA as u32;
            }
            assert_eq!(capacity, units.len() + 1);
            buffer[..units.len()].copy_from_slice(&units);
            buffer[units.len()] = 0;
            0
        })
        .unwrap();
        assert_eq!(result, text);
        assert_eq!(calls, 2);
    }

    #[test]
    fn rejects_oversized_or_invalid_utf16_msi_properties() {
        assert!(read_property(|_, size| {
            *size = 65_537;
            ERROR_MORE_DATA as u32
        })
        .is_err());
        assert!(read_property(|buffer, size| {
            let capacity = *size;
            *size = 1;
            if capacity == 0 {
                return ERROR_MORE_DATA as u32;
            }
            buffer[0] = 0xD800;
            buffer[1] = 0;
            0
        })
        .is_err());
        assert_eq!(read_property(|_, _| 5).unwrap_err().raw_os_error(), Some(5));
    }

    #[test]
    fn registry_cleanup_removes_only_owned_value_and_is_idempotent() {
        let fixture = Fixture::new();
        fixture.set("LazyWords", &format!("{MSI_EXE} "));
        fixture.set("OtherApp", "untouched");
        assert!(fixture.cleanup());
        assert_eq!(
            read_string(fixture.key.0, &wide("LazyWords")).unwrap(),
            None
        );
        assert_eq!(
            read_string(fixture.key.0, &wide("OtherApp"))
                .unwrap()
                .as_deref(),
            Some("untouched")
        );
        assert!(!fixture.cleanup());
    }

    #[test]
    fn registry_cleanup_preserves_nsis_and_missing_values() {
        let fixture = Fixture::new();
        assert!(!fixture.cleanup());
        fixture.set("LazyWords", &format!("{NSIS_EXE} "));
        assert!(!fixture.cleanup());
        assert_eq!(
            read_string(fixture.key.0, &wide("LazyWords"))
                .unwrap()
                .unwrap(),
            format!("{NSIS_EXE} ")
        );
    }

    #[test]
    fn registry_cleanup_ignores_non_string_values() {
        let fixture = Fixture::new();
        let bytes = [1u8, 2, 3, 4];
        assert_eq!(
            unsafe {
                RegSetValueExW(
                    fixture.key.0,
                    wide("LazyWords").as_ptr(),
                    0,
                    3,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                )
            },
            0
        );
        assert!(!fixture.cleanup());
        let mut size = 0;
        assert_eq!(
            unsafe {
                RegQueryValueExW(
                    fixture.key.0,
                    wide("LazyWords").as_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut size,
                )
            },
            0
        );
        assert_eq!(size, 4);
    }
}
