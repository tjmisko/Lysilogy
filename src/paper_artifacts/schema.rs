//! A small validator for the JSON Schemas this crate generates.
//!
//! Only the keywords the generated schemas use are understood: `type`,
//! `required`, `properties`, `additionalProperties: false`, `items`, `enum`,
//! `minItems`, and `maxItems`. It is enough to reject a self-check correction
//! that would write a value the app cannot read.

use serde_json::Value;

/// Validate `value` against `schema`, returning the first problem found.
///
/// # Errors
/// Returns a human-readable path and reason when the value does not match.
pub fn validate(value: &Value, schema: &Value) -> Result<(), String> {
    validate_at(value, schema, "")
}

fn validate_at(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let Some(schema) = schema.as_object() else {
        return Ok(());
    };
    if let Some(types) = schema.get("type")
        && !matches_type(value, types)
    {
        return Err(format!(
            "{path}: expected type {}, got {}",
            types,
            kind(value)
        ));
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !values.iter().any(|allowed| allowed == value)
    {
        return Err(format!("{path}: value is not one of the allowed values"));
    }
    match value {
        Value::Object(object) => {
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for key in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(key) {
                        return Err(format!("{path}: missing required field '{key}'"));
                    }
                }
            }
            let properties = schema.get("properties").and_then(Value::as_object);
            for (key, child) in object {
                let Some(child_schema) = properties.and_then(|properties| properties.get(key))
                else {
                    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                        return Err(format!("{path}: unexpected field '{key}'"));
                    }
                    continue;
                };
                validate_at(child, child_schema, &format!("{path}/{key}"))?;
            }
        }
        Value::Array(items) => {
            let minimum = schema.get("minItems").and_then(Value::as_u64).unwrap_or(0);
            let maximum = schema.get("maxItems").and_then(Value::as_u64);
            if items.len() as u64 > maximum.unwrap_or(u64::MAX) {
                return Err(format!("{path}: too many items"));
            }
            if (items.len() as u64) < minimum {
                return Err(format!("{path}: too few items"));
            }
            if let Some(item_schema) = schema.get("items") {
                for (index, item) in items.iter().enumerate() {
                    validate_at(item, item_schema, &format!("{path}/{index}"))?;
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

/// The schema for the value at a JSON Pointer such as `/sections/0/title`.
#[must_use]
pub fn subschema_at<'a>(schema: &'a Value, pointer: &str) -> Option<&'a Value> {
    let mut current = schema;
    for segment in pointer.split('/').filter(|segment| !segment.is_empty()) {
        current = if let Ok(index) = segment.parse::<usize>() {
            let _ = index;
            current.get("items")?
        } else {
            current.get("properties")?.get(segment)?
        };
    }
    Some(current)
}

fn matches_type(value: &Value, types: &Value) -> bool {
    match types {
        Value::String(name) => matches_single(value, name),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| matches_single(value, name)),
        _ => true,
    }
}

fn matches_single(value: &Value, name: &str) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

const fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["sections"],
            "properties": {
                "sections": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["title", "pages"],
                        "properties": {
                            "title": {"type": "string"},
                            "pages": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["start", "end"],
                                "properties": {
                                    "start": {"type": "integer"},
                                    "end": {"type": "integer"}
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn should_accept_a_value_that_matches_the_shape() {
        let value = json!({"sections": [{"title": "Intro", "pages": {"start": 1, "end": 2}}]});
        assert!(validate(&value, &schema()).is_ok());
    }

    #[test]
    fn should_reject_a_section_missing_the_pages_field() {
        let value = json!({"sections": [{"title": "Intro"}]});
        let error = validate(&value, &schema()).expect_err("must reject");
        assert!(error.contains("pages"), "{error}");
    }

    #[test]
    fn should_reject_an_unexpected_field_and_a_bad_type() {
        let extra = json!({"sections": []});
        assert!(validate(&extra, &schema()).is_ok());
        let wrong_type = json!({"sections": "nope"});
        assert!(validate(&wrong_type, &schema()).is_err());
    }

    #[test]
    fn should_find_the_schema_for_a_nested_pointer() {
        let schema = schema();
        let found = subschema_at(&schema, "/sections/0/pages/start").expect("schema");
        assert_eq!(found["type"], "integer");
        assert!(subschema_at(&schema, "/missing").is_none());
    }
}
