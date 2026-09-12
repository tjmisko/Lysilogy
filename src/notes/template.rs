use std::path::Path;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use super::{MAX_NOTES_BYTES, NotesError};

const DEFAULT_TEMPLATE: &str = "---\ndate: {{date}}\ntime: {{time}}\ntags:\n{{tags}}\n---\n\n## Sources\n- [{{source_title}}](<{{source_url}}>)\n";
const PLACEHOLDERS: [&str; 5] = ["date", "time", "tags", "source_title", "source_url"];

/// New-note defaults. Existing notes are never reformatted or reinitialized.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NoteTemplateConfig {
    pub date_format: String,
    pub time_format: String,
    pub tags: Vec<String>,
    pub template: String,
}

impl Default for NoteTemplateConfig {
    fn default() -> Self {
        Self {
            date_format: "YYYY-MM-DD".into(),
            time_format: "HH:mm".into(),
            tags: vec!["paper".into()],
            template: DEFAULT_TEMPLATE.into(),
        }
    }
}

impl NoteTemplateConfig {
    pub fn validate(&self) -> Result<(), NotesError> {
        compile_format(&self.date_format)?;
        compile_format(&self.time_format)?;
        if self.tags.len() > 64
            || self
                .tags
                .iter()
                .any(|tag| tag.is_empty() || tag.len() > 256)
        {
            return Err(invalid(
                "Use at most 64 nonempty tags, each at most 256 bytes.",
            ));
        }
        if self.template.len() > 64 * 1024 {
            return Err(invalid("The template must be at most 64 KiB."));
        }
        scan_template(&self.template, |_| Ok(String::new()))?;
        Ok(())
    }

    pub(super) fn render(
        &self,
        relative_pdf: &str,
        source_path: &Path,
        instant: DateTime<FixedOffset>,
    ) -> Result<String, NotesError> {
        self.validate()?;
        let date = instant
            .format(&compile_format(&self.date_format)?)
            .to_string();
        let time = instant
            .format(&compile_format(&self.time_format)?)
            .to_string();
        let tags = if self.tags.is_empty() {
            "  []".into()
        } else {
            self.tags
                .iter()
                .map(|tag| format!("  - {}", yaml_tag(tag)))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let title = Path::new(relative_pdf)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| invalid("The source PDF has no UTF-8 filename stem."))?;
        // Lexical absolutization does not open, resolve symlinks, or read the source.
        let absolute = std::path::absolute(source_path)?;
        let url = reqwest::Url::from_file_path(&absolute)
            .map_err(|()| invalid("The source PDF path cannot be represented as a file URL."))?;
        let text = scan_template(&self.template, |placeholder| {
            Ok(match placeholder {
                "date" => yaml_scalar(&date),
                "time" => {
                    if plain_clock(&time) {
                        time.clone()
                    } else {
                        yaml_scalar(&time)
                    }
                }
                "tags" => tags.clone(),
                "source_title" => markdown_label(title),
                "source_url" => url.to_string(),
                _ => unreachable!("scan_template validates placeholders"),
            })
        })?;
        if text.len() > MAX_NOTES_BYTES {
            return Err(NotesError::TooLarge);
        }
        Ok(text)
    }
}

fn scan_template(
    template: &str,
    mut expand: impl FnMut(&str) -> Result<String, NotesError>,
) -> Result<String, NotesError> {
    let mut rest = template;
    let mut result = String::with_capacity(template.len());
    while let Some(start) = rest.find("{{") {
        if rest[..start].contains("}}") {
            return Err(invalid("A closing }} must match an opening {{."));
        }
        result.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail
            .find("}}")
            .ok_or_else(|| invalid("An opening {{ must have a closing }}."))?;
        let placeholder = &tail[..end];
        if !PLACEHOLDERS.contains(&placeholder) {
            return Err(invalid(&format!(
                "Unknown placeholder {{{{{placeholder}}}}}; supported placeholders are date, time, tags, source_title, source_url."
            )));
        }
        result.push_str(&expand(placeholder)?);
        rest = &tail[end + 2..];
    }
    if rest.contains("}}") {
        return Err(invalid("A closing }} must match an opening {{."));
    }
    result.push_str(rest);
    Ok(result)
}

fn compile_format(format: &str) -> Result<String, NotesError> {
    if format.is_empty() || format.len() > 128 || format.chars().any(char::is_control) {
        return Err(invalid(
            "Date/time formats require 1–128 bytes without control characters.",
        ));
    }
    let tokens = [
        ("YYYY", "%Y"),
        ("YY", "%y"),
        ("MM", "%m"),
        ("DD", "%d"),
        ("HH", "%H"),
        ("hh", "%I"),
        ("mm", "%M"),
        ("ss", "%S"),
        ("ZZ", "%z"),
        ("M", "%-m"),
        ("D", "%-d"),
        ("H", "%-H"),
        ("h", "%-I"),
        ("m", "%-M"),
        ("s", "%-S"),
        ("A", "%p"),
        ("a", "%P"),
        ("Z", "%:z"),
    ];
    let mut result = String::new();
    let mut rest = format;
    while !rest.is_empty() {
        if let Some(literal) = rest.strip_prefix('[') {
            let end = literal
                .find(']')
                .ok_or_else(|| invalid("Format literal [ must end with ]."))?;
            result.push_str(&literal[..end].replace('%', "%%"));
            rest = &literal[end + 1..];
        } else if let Some((token, specifier)) =
            tokens.iter().find(|(token, _)| rest.starts_with(token))
        {
            result.push_str(specifier);
            rest = &rest[token.len()..];
        } else {
            let character = rest.chars().next().unwrap_or_default();
            if character.is_alphanumeric() || character == ']' || character == '%' {
                return Err(invalid(&format!(
                    "Unsupported date/time format near {rest:?}. Use YYYY, YY, MM, M, DD, D, HH, H, hh, h, mm, m, ss, s, A, a, Z, ZZ, punctuation, or [literal text]."
                )));
            }
            result.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    Ok(result)
}

fn yaml_scalar(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.'))
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "true"
                | "false"
                | "yes"
                | "no"
                | "on"
                | "off"
                | "null"
                | ".nan"
                | ".inf"
                | "-.inf"
                | "+.inf"
        )
        && text.parse::<f64>().is_err()
        && text != "-";
    if plain {
        text.to_owned()
    } else {
        serde_json::to_string(text).expect("strings serialize as JSON")
    }
}

fn yaml_tag(text: &str) -> String {
    // Tags are always strings, even when their spelling resembles a YAML number/date.
    if text.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        serde_json::to_string(text).expect("strings serialize as JSON")
    } else {
        yaml_scalar(text)
    }
}

fn plain_clock(text: &str) -> bool {
    let pieces = text.split(':').collect::<Vec<_>>();
    (2..=3).contains(&pieces.len())
        && pieces.iter().all(|piece| {
            (1..=2).contains(&piece.len()) && piece.chars().all(|ch| ch.is_ascii_digit())
        })
}

fn markdown_label(title: &str) -> String {
    title
        .chars()
        .flat_map(|ch| match ch {
            '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' => vec!['\\', ch],
            '\r' | '\n' | '\t' => vec![' '],
            _ => vec![ch],
        })
        .collect()
}

fn invalid(message: &str) -> NotesError {
    NotesError::InvalidTemplate(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-09-11T23:07:08-07:00").unwrap()
    }

    #[test]
    fn default_template_preserves_the_browser_local_date_and_time() {
        let text = NoteTemplateConfig::default()
            .render(
                "Author - Paper.pdf",
                Path::new("/workspace/local-articles/Author - Paper.pdf"),
                instant(),
            )
            .unwrap();
        assert_eq!(
            text,
            "---\ndate: 2026-09-11\ntime: 23:07\ntags:\n  - paper\n---\n\n## Sources\n- [Author - Paper](<file:///workspace/local-articles/Author%20-%20Paper.pdf>)\n"
        );
    }

    #[test]
    fn custom_formats_tags_and_template_are_validated_and_escaped() {
        let config = NoteTemplateConfig {
            date_format: "DD/MM/YYYY".into(),
            time_format: "hh:mm A Z".into(),
            tags: vec![
                "paper".into(),
                "topic: [AI]\nquoted \"tag\"".into(),
                "true".into(),
                "123".into(),
            ],
            template: "created: {{date}}\nat: {{time}}\ntags:\n{{tags}}\n{{source_title}}\n{{source_url}}\n".into(),
        };
        let text = config
            .render(
                "Research [v2]_日本語.pdf",
                Path::new("/papers/Research [v2]_日本語 #%.pdf"),
                instant(),
            )
            .unwrap();
        assert!(text.starts_with("created: 11/09/2026\nat: \"11:07 PM -07:00\"\n"));
        assert!(text.contains(
            "tags:\n  - paper\n  - \"topic: [AI]\\nquoted \\\"tag\\\"\"\n  - \"true\"\n  - \"123\""
        ));
        assert!(text.contains("Research \\[v2\\]\\_日本語\n"));
        assert!(text.contains("%E6%97%A5%E6%9C%AC%E8%AA%9E%20%23%25.pdf"));
    }

    #[test]
    fn invalid_formats_and_placeholders_fail_before_any_note_is_created() {
        for format in ["", "YYYY-qq", "%Y", "YYYY\nMM", "[unclosed"] {
            assert!(compile_format(format).is_err(), "{format}");
        }
        for template in ["{{unknown}}", "{{date}", "unmatched }}"] {
            assert!(
                NoteTemplateConfig {
                    template: template.into(),
                    ..NoteTemplateConfig::default()
                }
                .validate()
                .is_err()
            );
        }
        let config: NoteTemplateConfig = serde_json::from_str(r#"{"tags":[]}"#).unwrap();
        assert_eq!(config.date_format, "YYYY-MM-DD");
        assert!(
            config
                .render("x.pdf", Path::new("/x.pdf"), instant())
                .unwrap()
                .contains("tags:\n  []")
        );
    }
}
