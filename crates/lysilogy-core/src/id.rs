//! Opaque entity identifiers.
//!
//! Internally every entity is a `UUID`. Externally it is presented as a short,
//! URL-safe code: a single type prefix followed by the base64url encoding of the
//! 16 UUID bytes (22 characters, unpadded). The prefix makes an ID self-describing
//! and prevents one entity type's ID from being mistaken for another's.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::Uuid;

/// base64url alphabet (RFC 4648 §5) without padding.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encoded length of a 128-bit UUID in unpadded base64url.
const CODE_LEN: usize = 22;

/// Errors from parsing or encoding an entity identifier.
#[derive(Debug, thiserror::Error)]
pub enum IdError {
    #[error("identifier must be 22 characters after the prefix")]
    Length,
    #[error("identifier contains a character outside the base64url alphabet")]
    Character,
    #[error("identifier has non-zero padding bits")]
    Padding,
    #[error("identifier has an unexpected type prefix")]
    Prefix,
}

/// Encode the 16 bytes of a UUID as unpadded base64url.
#[allow(clippy::cast_possible_truncation)]
fn encode_uuid(uuid: &Uuid) -> String {
    let mut output = String::with_capacity(CODE_LEN);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in uuid.as_bytes() {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            let index = ((buffer >> bits) & 0x3f) as usize;
            output.push(ALPHABET[index] as char);
        }
        buffer &= (1u32 << bits) - 1;
    }
    if bits > 0 {
        let index = ((buffer << (6 - bits)) & 0x3f) as usize;
        output.push(ALPHABET[index] as char);
    }
    output
}

/// Decode an unpadded base64url code back into a UUID.
#[allow(clippy::cast_possible_truncation)]
fn decode_uuid(text: &str) -> Result<Uuid, IdError> {
    if text.len() != CODE_LEN {
        return Err(IdError::Length);
    }
    let mut bytes = [0u8; 16];
    let mut written = 0usize;
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err(IdError::Character),
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes[written] = ((buffer >> bits) & 0xff) as u8;
            written += 1;
        }
        buffer &= (1u32 << bits) - 1;
    }
    // 22 characters carry 132 bits; the trailing 4 bits of a 128-bit UUID are pad.
    if bits != 4 || buffer != 0 {
        return Err(IdError::Padding);
    }
    Ok(Uuid::from_bytes(bytes))
}

/// Generate a distinct `Uuid` newtype with a shared external codec.
macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            /// The single-character type prefix used in the external code.
            pub const PREFIX: char = $prefix;

            /// Mint a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wrap an existing [`Uuid`].
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// The underlying [`Uuid`].
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}{}", Self::PREFIX, encode_uuid(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let mut characters = text.chars();
                match characters.next() {
                    Some(character) if character == Self::PREFIX => {}
                    _ => return Err(IdError::Prefix),
                }
                Ok(Self(decode_uuid(characters.as_str())?))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(de::Error::custom)
            }
        }
    };
}

id_type!(
    /// Identifies a bibliographic [`Work`](crate::work::Work).
    WorkId,
    'w'
);
id_type!(
    /// Identifies one version or manifestation of a Work.
    WorkVersionId,
    'v'
);
id_type!(
    /// Identifies one identifier claim attached to a Work.
    IdentifierId,
    'i'
);
id_type!(
    /// Identifies a retained evidence payload from a provider or extraction.
    SourceRecordId,
    's'
);
id_type!(
    /// Identifies a bibliography entry inside a citing document version.
    EntryId,
    'e'
);
id_type!(
    /// Identifies a workspace.
    WorkspaceId,
    'k'
);
id_type!(
    /// Identifies a reader document (stable across file replacement).
    DocumentId,
    'd'
);
id_type!(
    /// Identifies one immutable version of a document's bytes.
    DocumentVersionId,
    'f'
);
id_type!(
    /// Identifies a stored blob.
    BlobId,
    'l'
);
id_type!(
    /// Identifies a citation assertion (one source's evidence for an edge).
    AssertionId,
    'c'
);
id_type!(
    /// Identifies an in-text citation occurrence.
    MentionId,
    'm'
);
id_type!(
    /// Identifies a relationship claim (a label on a citation edge).
    RelationshipClaimId,
    'r'
);
id_type!(
    /// Identifies a private workspace tag.
    TagId,
    't'
);
id_type!(
    /// Identifies a reading ramp.
    RampId,
    'p'
);
id_type!(
    /// Identifies one field claim (a sourced value for a Work's title, year, etc.).
    FieldClaimId,
    'a'
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_round_trips_every_uuid_shape() {
        for _ in 0..10_000 {
            let uuid = Uuid::new_v4();
            let code = encode_uuid(&uuid);
            assert_eq!(code.len(), CODE_LEN);
            assert!(code.is_ascii());
            assert_eq!(decode_uuid(&code).expect("decode"), uuid);
        }
    }

    #[test]
    fn codec_handles_extremes() {
        for uuid in [Uuid::nil(), Uuid::max()] {
            let code = encode_uuid(&uuid);
            assert_eq!(decode_uuid(&code).expect("decode"), uuid);
        }
    }

    #[test]
    fn id_display_and_parse_round_trip() {
        for _ in 0..1_000 {
            let id = WorkId::new();
            let text = id.to_string();
            assert_eq!(text.chars().next(), Some('w'));
            assert_eq!(text.parse::<WorkId>().expect("parse"), id);
        }
    }

    #[test]
    fn id_rejects_wrong_prefix_and_shape() {
        let id = WorkId::new();
        let text = id.to_string();
        let mut other = text.clone();
        other.replace_range(0..1, "d");
        assert!(matches!(other.parse::<WorkId>(), Err(IdError::Prefix)));
        assert!(matches!("wshort".parse::<WorkId>(), Err(IdError::Length)));
        let mut bad = text;
        bad.replace_range(1..2, "!");
        assert!(matches!(bad.parse::<WorkId>(), Err(IdError::Character)));
    }

    #[test]
    fn distinct_types_do_not_parse_each_others_codes() {
        let work = WorkId::new().to_string();
        assert!(work.parse::<DocumentId>().is_err());
        assert!(work.parse::<WorkId>().is_ok());
    }

    #[test]
    fn ids_serialize_as_public_codes() {
        let id = EntryId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, format!("\"{id}\""));
        let back: EntryId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }

    #[test]
    fn should_round_trip_tag_id_when_formatting_and_parsing() {
        let id = TagId::new();
        let text = id.to_string();
        assert_eq!(text.chars().next(), Some('t'));
        assert_eq!(text.parse::<TagId>().expect("parse"), id);
        let mut other = text;
        other.replace_range(0..1, "w");
        assert!(matches!(other.parse::<TagId>(), Err(IdError::Prefix)));
        assert!(WorkId::new().to_string().parse::<TagId>().is_err());
    }

    #[test]
    fn should_round_trip_ramp_id_when_formatting_and_parsing() {
        let id = RampId::new();
        let text = id.to_string();
        assert_eq!(text.chars().next(), Some('p'));
        assert_eq!(text.parse::<RampId>().expect("parse"), id);
        let mut other = text;
        other.replace_range(0..1, "r");
        assert!(matches!(other.parse::<RampId>(), Err(IdError::Prefix)));
        assert!(WorkId::new().to_string().parse::<RampId>().is_err());
    }
}
