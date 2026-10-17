//! Presentation-only localization. Source text is a stable gettext-style message ID.
//! Locale scopes never translate project content, execution arguments or console bytes.
use serde::{Deserialize, Serialize};
use std::{cell::Cell, collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en")]
    English,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    Chinese,
    English,
}
impl Language {
    pub fn resolve(self, system: Locale) -> Locale {
        match self {
            Self::System => system,
            Self::Chinese => Locale::Chinese,
            Self::English => Locale::English,
        }
    }
}
fn english() -> &'static BTreeMap<String, String> {
    static CATALOG: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("locales/en.json"))
            .expect("validated bundled language catalog")
    })
}
impl Locale {
    pub fn text(self, source: &str) -> &str {
        match self {
            Self::Chinese => source,
            Self::English => english().get(source).map(String::as_str).unwrap_or(source),
        }
    }
    /// Single-pass named interpolation: values containing braces are never interpreted.
    pub fn format(self, source: &str, values: &[(&str, String)]) -> String {
        let mut rest = self.text(source);
        let mut result = String::with_capacity(rest.len());
        while let Some(start) = rest.find('{') {
            result.push_str(&rest[..start]);
            let Some(end) = rest[start + 1..].find('}') else {
                result.push_str(&rest[start..]);
                return result;
            };
            let end = start + 1 + end;
            let key = &rest[start + 1..end];
            if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
                result.push_str(value);
            } else {
                result.push_str(&rest[start..=end]);
            }
            rest = &rest[end + 1..];
        }
        result.push_str(rest);
        result
    }
}
thread_local! { static CURRENT: Cell<Locale> = const { Cell::new(Locale::Chinese) }; }
pub struct Scope {
    previous: Locale,
    thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.set(self.previous);
    }
}
/// A frame-local scope also covers immediate detached viewports. Restoring on
/// drop isolates other application instances and parallel test threads.
pub fn scope(locale: Locale) -> Scope {
    Scope {
        previous: CURRENT.replace(locale),
        thread: std::marker::PhantomData,
    }
}
pub fn current() -> Locale {
    CURRENT.get()
}
pub fn tr(source: &str) -> &str {
    current().text(source)
}
pub fn tf(source: &str, values: &[(&str, String)]) -> String {
    current().format(source, values)
}

/// OS diagnostics are kept intact unless the complete message has a reviewed translation.
pub fn diagnostic(source: &str) -> std::borrow::Cow<'_, str> {
    if current() == Locale::Chinese {
        return std::borrow::Cow::Borrowed(source);
    }
    let translated = tr(source);
    if translated != source {
        return std::borrow::Cow::Borrowed(translated);
    }
    // Translate reviewed application prefixes only. File names, user values and
    // underlying OS/tool diagnostics remain byte-for-byte intact after the prefix.
    for prefix in [
        "本地API不可用: ",
        "配置解析失败（原文件保留）: ",
        "DevStart启动配置已被外部修改，请先重新读取启动脚本: ",
        "读取DevStart启动配置 ",
        "项目入口路径 ",
        "读取启动清单 ",
        "启动清单解析失败: ",
        "项目目录 ",
        "无效启动入口: ",
        "无效环境变量名称: ",
        "文件已被外部修改；",
        "缺少已编译主类 ",
    ] {
        if let Some(detail) = source.strip_prefix(prefix) {
            return std::borrow::Cow::Owned(format!("{}{detail}", tr(prefix)));
        }
    }
    std::borrow::Cow::Borrowed(source)
}

pub fn tf_count(source: &str, values: &[(&str, String)], count: usize) -> String {
    if current() == Locale::English
        && count == 1
        && let Some(singular) = english().get(&format!("{source}#one"))
    {
        return Locale::English.format(singular, values);
    }
    tf(source, values)
}

pub fn group_name(group: &crate::model::Group) -> &str {
    if group.id == 1 && group.name == "默认分组" {
        tr("默认分组")
    } else {
        &group.name
    }
}
/// Only generated external-process labels are localized; user launch names are untouched.
pub fn run_title(run: &crate::runtime::Run) -> std::borrow::Cow<'_, str> {
    if run.external.is_some()
        && let Some(project) = run.title.strip_suffix(" / 外部运行（参数未识别）")
    {
        return std::borrow::Cow::Owned(tf(
            "{project} / 外部运行（参数未识别）",
            &[("project", project.to_owned())],
        ));
    }
    std::borrow::Cow::Borrowed(&run.title)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn placeholders(text: &str) -> std::collections::BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|(key, _)| key))
            .collect()
    }
    #[test]
    fn english_catalog_is_complete_and_interpolation_contracts_match() {
        for (source, translated) in english() {
            assert!(!translated.trim().is_empty(), "empty translation: {source}");
            assert_eq!(
                placeholders(source),
                placeholders(translated),
                "placeholder mismatch: {source}"
            );
        }
        for source in [
            include_str!("ui.rs"),
            include_str!("ui_editor.rs"),
            include_str!("ui_settings.rs"),
            include_str!("ui_about.rs"),
            include_str!("ui_git.rs"),
            include_str!("ui_git_commit.rs"),
            include_str!("ui_git_remote.rs"),
            include_str!("ui_terminal.rs"),
            include_str!("ui_console_search.rs"),
            include_str!("ui_console_render.rs"),
        ] {
            let source = source.split("#[cfg(test)]").next().unwrap();
            let mut offset = 0;
            while let Some(start) = source[offset..].find('"') {
                let start = offset + start;
                let mut end = start + 1;
                let bytes = source.as_bytes();
                while end < bytes.len() {
                    if bytes[end] == b'\\' {
                        end += 2;
                        continue;
                    }
                    if bytes[end] == b'"' {
                        break;
                    }
                    end += 1;
                }
                if end >= bytes.len() {
                    break;
                }
                if let Ok(message) = serde_json::from_str::<String>(&source[start..=end])
                    && message
                        .chars()
                        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
                {
                    let prefix = source[..start].trim_end();
                    assert!(
                        ["tr(", "tf(", "tf_count(", "==", "!="]
                            .iter()
                            .any(|marker| prefix.ends_with(marker)),
                        "UI literal bypasses localization: {message}"
                    );
                    assert!(
                        english().contains_key(&message),
                        "missing UI translation: {message}"
                    );
                }
                offset = end + 1;
            }
        }
    }
    #[test]
    fn scopes_restore_locale_and_values_are_never_translated_or_reinterpreted() {
        let _english = scope(Locale::English);
        assert_eq!(tr("项目"), "Projects");
        assert_eq!(tf_count("{0} 行", &[("0", "1".into())], 1), "1 line");
        assert_eq!(tf_count("{0} 行", &[("0", "2".into())], 2), "2 lines");
        assert_eq!(
            diagnostic("项目入口路径 ~/用户/{name}: missing"),
            "Project entry path ~/用户/{name}: missing"
        );
        let user = "我的项目 {0} · 运行中";
        assert_eq!(
            tf("{0} · 失败", &[("0", user.to_owned())]),
            format!("{user} · Failed")
        );
        {
            let _chinese = scope(Locale::Chinese);
            assert_eq!(tr("项目"), "项目");
        }
        assert_eq!(tr("项目"), "Projects");
        let custom = crate::model::Group {
            id: 9,
            name: "默认分组".into(),
        };
        assert_eq!(group_name(&custom), "默认分组");
        assert_eq!(Language::System.resolve(Locale::Chinese), Locale::Chinese);
        assert_eq!(Language::English.resolve(Locale::Chinese), Locale::English);
        assert_eq!(Language::Chinese.resolve(Locale::English), Locale::Chinese);
    }
    #[test]
    fn localized_elapsed_time_preserves_units_and_seconds() {
        assert_eq!(
            crate::time_display::duration_for_locale(93784, Locale::English),
            "1d 2h 3m 4s"
        );
        assert_eq!(
            crate::time_display::duration_for_locale(60, Locale::English),
            "1m 0s"
        );
        assert_eq!(
            crate::time_display::duration_for_locale(0, Locale::English),
            "0s"
        );
        assert_eq!(
            crate::time_display::duration_for_locale(93784, Locale::Chinese),
            "1天2小时3分钟4秒"
        );
    }
}
