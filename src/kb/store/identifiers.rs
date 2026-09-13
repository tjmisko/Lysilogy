//! Admission accepts canonical identifiers only; lookup never infers an identity merge.
use super::{Result, StoreError};
use crate::kb::{Identifier, PersonIdentifier};

pub(super) fn work_key(identifier: &Identifier) -> Result<String> {
    let key = identifier.key();
    let parsed = Identifier::parse(&key).map_err(|error| StoreError::Invalid(error.message))?;
    if parsed != *identifier {
        return Err(StoreError::Invalid("noncanonical Work identifier".into()));
    }
    Ok(key)
}

pub(super) fn person_key(identifier: &PersonIdentifier) -> Result<String> {
    let (scheme, value, valid) = match identifier {
        PersonIdentifier::Orcid(value) => ("orcid", value, valid_orcid(value)),
        PersonIdentifier::OpenalexAuthor(value) => (
            "openalex_author",
            value,
            value.strip_prefix('A').is_some_and(canonical_number),
        ),
        PersonIdentifier::SemanticScholarAuthor(value) => {
            ("semantic_scholar_author", value, canonical_number(value))
        }
    };
    if !valid {
        return Err(StoreError::Invalid(format!(
            "noncanonical Person {scheme} identifier"
        )));
    }
    Ok(format!("{scheme}:{value}"))
}

fn canonical_number(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_orcid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    let mut total = 0_u32;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if matches!(index, 4 | 9 | 14) {
            if byte != b'-' {
                return false;
            }
        } else if index < 18 {
            if !byte.is_ascii_digit() {
                return false;
            }
            total = (total + u32::from(byte - b'0')) * 2;
        }
    }
    let check = (12 - total % 11) % 11;
    let expected = if check == 10 {
        b'X'
    } else {
        b'0' + u8::try_from(check).expect("single check digit")
    };
    bytes[18] == expected
}
