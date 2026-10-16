//! Native preferred UI languages, independent of terminal locale on desktop OSes.
use crate::i18n::Locale;

pub fn preferred<'a>(languages: impl IntoIterator<Item = &'a str>) -> Locale {
    for language in languages {
        let base = language.split(['-', '_', '.', '@']).next().unwrap_or("");
        if base.eq_ignore_ascii_case("zh") {
            return Locale::Chinese;
        }
        if base.eq_ignore_ascii_case("en") {
            return Locale::English;
        }
    }
    Locale::English
}
pub fn detect() -> Locale {
    let languages = native_languages();
    preferred(languages.iter().map(String::as_str))
}
#[cfg(target_os = "macos")]
fn native_languages() -> Vec<String> {
    use std::ffi::{CStr, c_char, c_void};
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> *const c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(value: *const c_void);
    }
    // Copy returns an owned CFArray of borrowed CFStrings. Every string is copied
    // while the array is alive; its ownership is released exactly once below.
    unsafe {
        let languages = CFLocaleCopyPreferredLanguages();
        if languages.is_null() {
            return vec![];
        }
        let mut result = vec![];
        for index in 0..CFArrayGetCount(languages).min(32) {
            let value = CFArrayGetValueAtIndex(languages, index);
            let mut buffer = [0 as c_char; 256];
            if !value.is_null()
                && CFStringGetCString(
                    value,
                    buffer.as_mut_ptr(),
                    buffer.len() as isize,
                    0x0800_0100,
                ) != 0
            {
                result.push(
                    CStr::from_ptr(buffer.as_ptr())
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        CFRelease(languages);
        result
    }
}
#[cfg(target_os = "windows")]
fn native_languages() -> Vec<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserPreferredUILanguages(
            flags: u32,
            count: *mut u32,
            buffer: *mut u16,
            length: *mut u32,
        ) -> i32;
    }
    let (mut count, mut length) = (0, 0);
    // Windows exposes BCP-47 language names as a bounded UTF-16 multistring.
    unsafe {
        if GetUserPreferredUILanguages(8, &mut count, std::ptr::null_mut(), &mut length) == 0
            || !(1..=4096).contains(&length)
        {
            return vec![];
        }
        let mut buffer = vec![0; length as usize];
        if GetUserPreferredUILanguages(8, &mut count, buffer.as_mut_ptr(), &mut length) == 0 {
            return vec![];
        }
        buffer
            .split(|ch| *ch == 0)
            .filter(|name| !name.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_languages() -> Vec<String> {
    for name in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            return value.split(':').map(str::to_owned).collect();
        }
    }
    vec![]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_ordered_supported_languages_and_common_region_spellings() {
        assert_eq!(preferred(["zh_CN.UTF-8"]), Locale::Chinese);
        assert_eq!(preferred(["fr-FR", "en-US", "zh-CN"]), Locale::English);
        assert_eq!(preferred(["fr", "zh-Hans-CN", "en"]), Locale::Chinese);
        assert_eq!(preferred(["EN_gb"]), Locale::English);
        assert_eq!(preferred(["ja", "C"]), Locale::English);
        assert_eq!(preferred([]), Locale::English);
    }
}
