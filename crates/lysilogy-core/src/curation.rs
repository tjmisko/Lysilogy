//! Validation for private curation input: tag names, tag colors, and mark notes.

/// Default tag colors, assigned in order so new tags are visually distinct.
pub const TAG_PALETTE: [&str; 8] = [
    "#4e79a7", "#f28e2b", "#59a14f", "#e15759", "#76b7b2", "#edc948", "#b07aa1", "#9c755f",
];

const TAG_NAME_MAX_CHARS: usize = 48;
const MARK_NOTE_MAX_CHARS: usize = 2000;

/// Why curation input was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CurationError {
    #[error("tag name must not be empty")]
    EmptyName,
    #[error("tag name must be at most {TAG_NAME_MAX_CHARS} characters")]
    NameTooLong,
    #[error("tag name must not contain control characters")]
    NameControlCharacter,
    #[error("color must be #rgb or #rrggbb hexadecimal")]
    InvalidColor,
    #[error("note must be at most {MARK_NOTE_MAX_CHARS} characters")]
    NoteTooLong,
}

impl CurationError {
    /// The machine-readable API error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptyName | Self::NameTooLong | Self::NameControlCharacter => "invalid_tag_name",
            Self::InvalidColor => "invalid_color",
            Self::NoteTooLong => "invalid_note",
        }
    }
}

/// A validated tag name: trimmed, single-spaced, 1 to 48 characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagName(String);

impl TagName {
    /// Trim, collapse whitespace runs to one space, and validate.
    pub fn parse(text: &str) -> Result<Self, CurationError> {
        if text.chars().any(char::is_control) {
            return Err(CurationError::NameControlCharacter);
        }
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            return Err(CurationError::EmptyName);
        }
        if collapsed.chars().count() > TAG_NAME_MAX_CHARS {
            return Err(CurationError::NameTooLong);
        }
        Ok(Self(collapsed))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated tag color, always lower-case `#rrggbb`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagColor(String);

impl TagColor {
    /// Accept `#rgb` or `#rrggbb` in any case and normalize to `#rrggbb`.
    pub fn parse(text: &str) -> Result<Self, CurationError> {
        let Some(digits) = text.strip_prefix('#') else {
            return Err(CurationError::InvalidColor);
        };
        if !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
            return Err(CurationError::InvalidColor);
        }
        let lower = digits.to_ascii_lowercase();
        match lower.len() {
            3 => {
                let mut expanded = String::with_capacity(7);
                expanded.push('#');
                for digit in lower.chars() {
                    expanded.push(digit);
                    expanded.push(digit);
                }
                Ok(Self(expanded))
            }
            6 => Ok(Self(format!("#{lower}"))),
            _ => Err(CurationError::InvalidColor),
        }
    }

    /// The palette color at `index`, wrapping around.
    #[must_use]
    pub fn from_palette(index: usize) -> Self {
        Self(TAG_PALETTE[index % TAG_PALETTE.len()].to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Trim a mark note; empty becomes `None`; more than 2,000 characters is rejected.
pub fn parse_mark_note(note: Option<&str>) -> Result<Option<String>, CurationError> {
    let Some(trimmed) = note.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MARK_NOTE_MAX_CHARS {
        return Err(CurationError::NoteTooLong);
    }
    Ok(Some(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TagKind, WorkMark};

    #[test]
    fn should_collapse_whitespace_when_parsing_tag_name() {
        let name = TagName::parse("  deep   learning \u{a0} methods ").expect("name");
        assert_eq!(name.as_str(), "deep learning methods");
    }

    #[test]
    fn should_reject_empty_long_and_control_character_tag_names() {
        assert_eq!(TagName::parse(""), Err(CurationError::EmptyName));
        assert_eq!(TagName::parse("   "), Err(CurationError::EmptyName));
        assert_eq!(
            TagName::parse(&"a".repeat(49)),
            Err(CurationError::NameTooLong)
        );
        assert_eq!(
            TagName::parse("bad\u{0}name"),
            Err(CurationError::NameControlCharacter)
        );
        assert_eq!(
            TagName::parse("line\nbreak"),
            Err(CurationError::NameControlCharacter)
        );
        assert_eq!(
            TagName::parse("tab\there"),
            Err(CurationError::NameControlCharacter)
        );
        assert_eq!(CurationError::EmptyName.code(), "invalid_tag_name");
        assert_eq!(CurationError::NameTooLong.code(), "invalid_tag_name");
    }

    #[test]
    fn should_count_characters_not_bytes_when_limiting_tag_name() {
        assert!(TagName::parse(&"é".repeat(48)).is_ok());
        assert_eq!(
            TagName::parse(&"é".repeat(49)),
            Err(CurationError::NameTooLong)
        );
    }

    #[test]
    fn should_expand_short_hex_and_lowercase_when_parsing_color() {
        assert_eq!(TagColor::parse("#ABC").expect("short").as_str(), "#aabbcc");
        assert_eq!(
            TagColor::parse("#4E79A7").expect("long").as_str(),
            "#4e79a7"
        );
        assert_eq!(TagColor::parse("#0f0").expect("short").as_str(), "#00ff00");
    }

    #[test]
    fn should_reject_named_colors_and_bad_hex_when_parsing_color() {
        for bad in [
            "red",
            "#12345",
            "#ggg",
            "rgb(1,2,3)",
            "",
            "#",
            "4e79a7",
            "#4e79a7ff",
            " #abc",
            "#éé",
        ] {
            assert_eq!(TagColor::parse(bad), Err(CurationError::InvalidColor), "{bad:?}");
        }
        assert_eq!(CurationError::InvalidColor.code(), "invalid_color");
    }

    #[test]
    fn should_wrap_palette_index_when_choosing_default_color() {
        assert_eq!(TagColor::from_palette(0).as_str(), TAG_PALETTE[0]);
        assert_eq!(TagColor::from_palette(7).as_str(), TAG_PALETTE[7]);
        assert_eq!(TagColor::from_palette(8).as_str(), TAG_PALETTE[0]);
        assert_eq!(TagColor::from_palette(19).as_str(), TAG_PALETTE[3]);
        for color in TAG_PALETTE {
            assert_eq!(TagColor::parse(color).expect("palette").as_str(), color);
        }
    }

    #[test]
    fn should_drop_empty_note_and_reject_long_note_when_parsing() {
        assert_eq!(parse_mark_note(None), Ok(None));
        assert_eq!(parse_mark_note(Some("  \n ")), Ok(None));
        assert_eq!(
            parse_mark_note(Some("  keep this ")),
            Ok(Some("keep this".to_owned()))
        );
        assert!(parse_mark_note(Some(&"é".repeat(2000))).is_ok());
        assert_eq!(
            parse_mark_note(Some(&"é".repeat(2001))),
            Err(CurationError::NoteTooLong)
        );
        assert_eq!(CurationError::NoteTooLong.code(), "invalid_note");
    }

    #[test]
    fn should_round_trip_work_mark_and_tag_kind_when_parsing_strings() {
        for mark in [WorkMark::Pinned, WorkMark::Downplayed, WorkMark::Hidden] {
            assert_eq!(mark.as_str().parse::<WorkMark>().expect("mark"), mark);
        }
        for kind in [TagKind::Topic, TagKind::Status] {
            assert_eq!(kind.as_str().parse::<TagKind>().expect("kind"), kind);
        }
        assert!("Pinned".parse::<WorkMark>().is_err());
        assert!("public".parse::<TagKind>().is_err());
        assert_eq!(
            serde_json::to_string(&WorkMark::Downplayed).expect("json"),
            "\"downplayed\""
        );
    }
}
