//! 内置中英词典；先匹配完整模板，再保留动态字段原样，避免翻译文件名或路径。
use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};

pub static CATALOG: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("translations.json")).expect("内置翻译词典格式错误")
});

struct Template {
    pattern: Regex,
    english: String,
    weight: usize,
    translated_fields: Vec<bool>,
}
static PLACEHOLDERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^{}]*\}").unwrap());
static TEMPLATES: LazyLock<Vec<Template>> = LazyLock::new(|| {
    let placeholders = &*PLACEHOLDERS;
    let mut templates = Vec::new();
    for (chinese, english) in CATALOG.iter() {
        if !placeholders.is_match(chinese) {
            continue;
        }
        let mut pattern = "(?s)^".to_string();
        let mut end = 0;
        let mut weight = 0;
        let mut translated_fields = Vec::new();
        for placeholder in placeholders.find_iter(chinese) {
            pattern.push_str(&regex::escape(&chinese[end..placeholder.start()]));
            pattern.push_str("(.*?)");
            weight += placeholder.start() - end;
            end = placeholder.end();
            let name = placeholder
                .as_str()
                .trim_matches(['{', '}'])
                .split(':')
                .next()
                .unwrap_or("");
            translated_fields.push(matches!(name, "error" | "e" | "ownership"));
        }
        pattern.push_str(&regex::escape(&chinese[end..]));
        pattern.push('$');
        weight += chinese.len() - end;
        templates.push(Template {
            pattern: Regex::new(&pattern).unwrap(),
            english: english.clone(),
            weight,
            translated_fields,
        });
    }
    templates.sort_by_key(|t| std::cmp::Reverse(t.weight));
    templates
});

pub fn translate(value: &str, english: bool) -> String {
    if !english {
        return value.to_owned();
    }
    if let Some(result) = CATALOG.get(value) {
        return result.clone();
    }
    let placeholders = &*PLACEHOLDERS;
    for template in TEMPLATES.iter() {
        if let Some(captures) = template.pattern.captures(value) {
            let mut output = String::new();
            let mut end = 0;
            for (index, placeholder) in placeholders.find_iter(&template.english).enumerate() {
                output.push_str(&template.english[end..placeholder.start()]);
                let value = captures.get(index + 1).map_or("", |m| m.as_str());
                if template.translated_fields.get(index) == Some(&true) {
                    output.push_str(&translate(value, true));
                } else {
                    output.push_str(value);
                }
                end = placeholder.end();
            }
            output.push_str(&template.english[end..]);
            return output;
        }
    }
    // 多行记录逐行处理，动态名称、路径和第三方元信息不会做片段替换。
    if value.contains('\n') {
        return value
            .split('\n')
            .map(|line| translate(line, true))
            .collect::<Vec<_>>()
            .join("\n");
    }
    value.to_owned()
}

/// 在索引后台线程预热模板，避免第一次切换语言阻塞界面。
pub fn warm_up() {
    LazyLock::force(&CATALOG);
    LazyLock::force(&TEMPLATES);
}
