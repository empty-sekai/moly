//! Structural preflight for extracted conversation rows. Parsers keep their
//! compact typed construction, but only receive rows proven safe to construct.

use serde_json::Value;

const GENERAL_OPS: &[&str] = &[
    "look_at_body",
    "wait_time",
    "label",
    "voice",
    "change_npc_eye",
    "change_npc_mouth",
    "change_animation",
    "text",
    "wait_click",
    "emoticon",
    "hide_emoticon",
    "show_talk_window",
    "hide_talk_window",
    "wait_time_on_auto_mode",
];
const FIXTURE_OPS: &[&str] = &[
    "look_at_body",
    "wait_time",
    "label",
    "voice",
    "change_npc_eye",
    "change_npc_mouth",
    "change_animation",
    "text",
    "wait_click",
    "emoticon",
    "hide_emoticon",
    "show_talk_window",
    "hide_talk_window",
    "wait_time_on_auto_mode",
    "look_at_fixture",
    "look_at_to_npc",
    "fixture_voice",
    "change_fixture_character_eye",
    "change_fixture_character_mouth",
    "change_fixture_timeline",
    "show_fixture_emoticon",
    "play_fixture_gimmick",
    "stop_fixture_gimmick",
];

pub(crate) fn general_row(row: &Value) -> Result<(), String> {
    let id = i32_integer(row, "talkId")?;
    for field in ["lua"] {
        string(row, field)?;
    }
    for field in ["siteGroupId", "termId"] {
        i32_integer(row, field)?;
    }
    let conditions = array(row, "conditions")?;
    if conditions.iter().any(|value| !value.is_string()) {
        return Err(format!("talk {id}: conditions contains a non-string"));
    }
    let values = array(row, "conditionValues")?;
    if conditions.len() != values.len() {
        return Err(format!(
            "talk {id}: conditionValues length does not match conditions"
        ));
    }
    for (index, (condition, value)) in conditions.iter().zip(values).enumerate() {
        let kind = value
            .get("conditionType")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("talk {id}: condition {index} has no string conditionType"))?;
        if Some(kind) != condition.as_str() {
            return Err(format!("talk {id}: condition {index} type is not paired"));
        }
        match value.get("conditionTypeValue") {
            Some(value)
                if value.is_null()
                    || value
                        .as_i64()
                        .is_some_and(|value| i32::try_from(value).is_ok()) => {}
            _ => return Err(format!("talk {id}: condition {index} has an invalid value")),
        }
    }
    tweet(row, id, true)?;
    voices(row, id)?;
    let steps = array(row, "steps")?;
    for (index, step) in steps.iter().enumerate() {
        general_step(step).map_err(|reason| format!("talk {id}, step {index}: {reason}"))?;
    }
    Ok(())
}

pub(crate) fn fixture_row(row: &Value) -> Result<(), String> {
    let id = i32_integer(row, "talkId")?;
    string(row, "lua")?;
    for field in ["form", "siteGroupId", "termId", "conditionGroupId"] {
        i32_integer(row, field)?;
    }
    for field in ["fixtureIds", "unitIds"] {
        let values = array(row, field)?;
        if values.is_empty() {
            return Err(format!("talk {id}: {field} is empty"));
        }
        for value in values {
            positive_i32_value(value).map_err(|reason| format!("talk {id}: {field} {reason}"))?;
        }
    }
    for (index, pair) in array(row, "pairs")?.iter().enumerate() {
        let Some(pair) = pair.as_array() else {
            return Err(format!("talk {id}: pair {index} is not an array"));
        };
        if pair.len() != 2 {
            return Err(format!("talk {id}: pair {index} is not two integers"));
        }
        for value in pair {
            positive_i32_value(value)
                .map_err(|reason| format!("talk {id}: pair {index} {reason}"))?;
        }
    }
    if array(row, "pairs")?.is_empty() {
        return Err(format!("talk {id}: pairs is empty"));
    }
    tweet(row, id, false)?;
    voices(row, id)?;
    let steps = array(row, "steps")?;
    for (index, step) in steps.iter().enumerate() {
        fixture_step(step).map_err(|reason| format!("talk {id}, step {index}: {reason}"))?;
    }
    Ok(())
}

fn tweet(row: &Value, id: i64, strict_strings: bool) -> Result<(), String> {
    let tweet = row
        .get("tweet")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("talk {id}: missing tweet object"))?;
    if tweet
        .get("id")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .is_none()
    {
        return Err(format!("talk {id}: tweet has no integer id"));
    }
    if strict_strings {
        for field in ["text", "motion", "eye", "mouth"] {
            if tweet.get(field).and_then(Value::as_str).is_none() {
                return Err(format!("talk {id}: tweet.{field} is not a string"));
            }
        }
    }
    Ok(())
}

fn voices(row: &Value, id: i64) -> Result<(), String> {
    if let Some(voices) = row.get("voices") {
        let voices = voices
            .as_array()
            .ok_or_else(|| format!("talk {id}: voices is not an array"))?;
        if voices.iter().any(|value| !value.is_string()) {
            return Err(format!("talk {id}: voices contains a non-string"));
        }
    }
    Ok(())
}

fn general_step(step: &Value) -> Result<(), String> {
    let op = operation(step, GENERAL_OPS)?;
    match op {
        "look_at_body" => {
            who(step, "who")?;
            who(step, "target")?;
            nonnegative_number(step, "duration")?;
        }
        "wait_time" | "wait_time_on_auto_mode" => {
            nonnegative_number(step, "seconds")?;
        }
        "label" | "text" => {
            string(step, if op == "label" { "name" } else { "text" })?;
        }
        "voice" => {
            string(step, "channel")?;
            string(step, "cue")?;
            who(step, "who")?;
        }
        "change_npc_eye" | "change_npc_mouth" => {
            who(step, "who")?;
            string(step, "pattern")?;
            string(step, "alias")?;
            crate::delayed_faces::delay_seconds(step)?;
        }
        "change_animation" => {
            who(step, "who")?;
            string(step, "motion")?;
            string(step, "alias")?;
            optional_number(step, "speed")?;
            optional_number(step, "playbackSpeed")?;
            optional_bool(step, "playEndMotion")?;
        }
        "emoticon" => {
            who(step, "who")?;
            string(step, "name")?;
            string(step, "alias")?;
            optional_nonnegative_number(step, "showSeconds")?;
        }
        "hide_emoticon" => {
            who(step, "who")?;
        }
        _ => {}
    }
    Ok(())
}

fn fixture_step(step: &Value) -> Result<(), String> {
    let op = operation(step, FIXTURE_OPS)?;
    let referents: &[&str] = match op {
        "look_at_body" => &["who", "target"],
        "voice" => &["who"],
        "change_npc_eye" | "change_npc_mouth" | "change_animation" | "emoticon"
        | "hide_emoticon" => &["who"],
        "look_at_fixture" => &["fixture"],
        "look_at_to_npc" => &["who"],
        _ => &[],
    };
    for field in referents {
        referent_i32_number(step, field)?;
    }
    let fixture_ids: &[&str] = match op {
        "look_at_to_npc" => &["fixture"],
        "fixture_voice"
        | "change_fixture_character_eye"
        | "change_fixture_character_mouth"
        | "change_fixture_timeline"
        | "show_fixture_emoticon" => &["fixture"],
        _ => &[],
    };
    for field in fixture_ids {
        positive_i32_number(step, field)?;
    }
    let strings: &[&str] = match op {
        "label" => &["name"],
        "text" => &["text"],
        "voice" => &["channel", "cue"],
        "change_npc_eye" | "change_npc_mouth" => &["pattern", "alias"],
        "change_animation" => &["motion", "alias"],
        "emoticon" | "show_fixture_emoticon" => &["name", "alias"],
        "fixture_voice" => &["cue"],
        "change_fixture_character_eye" | "change_fixture_character_mouth" => &["pattern", "alias"],
        "change_fixture_timeline" => &["name"],
        "play_fixture_gimmick" | "stop_fixture_gimmick" => &["fixture"],
        _ => &[],
    };
    for field in strings {
        string(step, field)?;
    }
    for field in ["speed", "playbackSpeed", "blend"] {
        optional_nonnegative_number(step, field)?;
    }
    if matches!(op, "change_npc_eye" | "change_npc_mouth") {
        crate::delayed_faces::delay_seconds(step)?;
    }
    match op {
        "wait_time" | "wait_time_on_auto_mode" => {
            nonnegative_number(step, "seconds")?;
        }
        "look_at_body" | "look_at_to_npc" => {
            nonnegative_number(step, "duration")?;
        }
        "look_at_fixture" => optional_nonnegative_number(step, "who")?,
        "change_fixture_timeline" => optional_nonnegative_number(step, "value")?,
        "emoticon" | "show_fixture_emoticon" => {
            optional_nonnegative_number(step, "showSeconds")?;
        }
        "stop_fixture_gimmick" => {
            nonnegative_number(step, "name")?;
        }
        _ => {}
    }
    if op == "wait_time" {
        optional_bool(step, "auto")?;
    }
    if op == "change_animation" {
        optional_bool(step, "playEndMotion")?;
    }
    Ok(())
}

fn operation<'a>(step: &'a Value, known: &[&str]) -> Result<&'a str, String> {
    let op = step
        .get("op")
        .and_then(Value::as_str)
        .ok_or("missing string op")?;
    known
        .contains(&op)
        .then_some(op)
        .ok_or_else(|| format!("unsupported op {op:?}"))
}

fn who(row: &Value, field: &str) -> Result<(), String> {
    let Some(value) = row.get(field) else {
        return Ok(());
    };
    if value.is_string() {
        return Ok(());
    }
    let Some(number) = value.as_f64() else {
        return Err(format!("{field} is not a referent"));
    };
    if number == 0.
        || (number.is_finite() && number > 0. && number.fract() == 0. && number <= u32::MAX as f64)
    {
        Ok(())
    } else {
        Err(format!("{field} is not an integral referent"))
    }
}

fn i32_integer(row: &Value, field: &str) -> Result<i64, String> {
    row.get(field)
        .and_then(Value::as_i64)
        .filter(|value| i32::try_from(*value).is_ok())
        .ok_or_else(|| format!("{field} is not an i32 integer"))
}
fn positive_i32_value(value: &Value) -> Result<i32, &'static str> {
    value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or("contains a non-positive or out-of-range integer")
}
fn positive_i32_number(row: &Value, field: &str) -> Result<(), String> {
    let value = number(row, field)?;
    if value > 0.0 && value.fract() == 0.0 && value <= i32::MAX as f64 {
        Ok(())
    } else {
        Err(format!("{field} is not a positive i32 referent"))
    }
}
fn referent_i32_number(row: &Value, field: &str) -> Result<(), String> {
    let value = number(row, field)?;
    if value >= 0.0 && value.fract() == 0.0 && value <= i32::MAX as f64 {
        Ok(())
    } else {
        Err(format!("{field} is not a nonnegative i32 referent"))
    }
}
fn string<'a>(row: &'a Value, field: &str) -> Result<&'a str, String> {
    row.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} is not a string"))
}
fn number(row: &Value, field: &str) -> Result<f64, String> {
    row.get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{field} is not a finite number"))
}
fn nonnegative_number(row: &Value, field: &str) -> Result<f64, String> {
    number(row, field).and_then(|value| {
        (value >= 0.0)
            .then_some(value)
            .ok_or_else(|| format!("{field} is negative"))
    })
}
fn optional_number(row: &Value, field: &str) -> Result<(), String> {
    match row.get(field) {
        None | Some(Value::Null) => Ok(()),
        Some(value) if value.as_f64().is_some_and(f64::is_finite) => Ok(()),
        _ => Err(format!("{field} is not a finite number")),
    }
}
fn optional_nonnegative_number(row: &Value, field: &str) -> Result<(), String> {
    optional_number(row, field)?;
    if let Some(value) = row.get(field).and_then(Value::as_f64) {
        if value < 0.0 {
            return Err(format!("{field} is negative"));
        }
    }
    Ok(())
}
fn optional_bool(row: &Value, field: &str) -> Result<(), String> {
    match row.get(field) {
        None | Some(Value::Null) | Some(Value::Bool(_)) => Ok(()),
        _ => Err(format!("{field} is not a boolean")),
    }
}
fn array<'a>(row: &'a Value, field: &str) -> Result<&'a Vec<Value>, String> {
    row.get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field} is not an array"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_tweet_is_a_recoverable_row_error() {
        let row = json!({"talkId": 9, "lua": "x", "form": 2, "siteGroupId": 1,
            "termId": 1, "conditionGroupId": 1, "fixtureIds": [157], "unitIds": [1],
            "pairs": [[157, 1]], "steps": []});
        assert!(fixture_row(&row).unwrap_err().contains("missing tweet"));
    }

    #[test]
    fn unknown_operation_is_quarantined() {
        let row = json!({"talkId": 9, "lua": "x", "form": 2, "siteGroupId": 1,
            "termId": 1, "conditionGroupId": 1, "fixtureIds": [157], "unitIds": [1],
            "pairs": [[157, 1]], "tweet": {"id": 1}, "steps": [{"op": "future_op"}]});
        assert!(fixture_row(&row).unwrap_err().contains("unsupported op"));
    }

    #[test]
    fn overflowing_integer_is_quarantined_before_i32_construction() {
        let row = json!({"talkId": i64::MAX, "lua": "x", "siteGroupId": 1,
            "termId": 1, "conditions": [], "conditionValues": [],
            "tweet": {"id": 1, "text": "", "motion": "", "eye": "", "mouth": ""},
            "steps": []});
        assert!(general_row(&row).unwrap_err().contains("i32"));
    }

    #[test]
    fn fixture_requires_a_real_nonempty_cast_and_pairing() {
        let row = json!({"talkId": 9, "lua": "x", "form": 2, "siteGroupId": 1,
            "termId": 1, "conditionGroupId": 1, "fixtureIds": [157], "unitIds": [],
            "pairs": [], "tweet": {"id": 1}, "steps": []});
        assert!(fixture_row(&row).unwrap_err().contains("unitIds is empty"));
    }

    #[test]
    fn nonfinite_or_negative_delays_are_quarantined() {
        let base = json!({"talkId": 9, "lua": "x", "siteGroupId": 1, "termId": 1,
            "conditions": [], "conditionValues": [],
            "tweet": {"id": 1, "text": "", "motion": "", "eye": "", "mouth": ""},
            "steps": [{"op": "change_npc_eye", "who": 1, "pattern": "x", "alias": "", "delaySeconds": "Infinity"}]});
        assert!(general_row(&base).unwrap_err().contains("delaySeconds"));
        let mut negative = base;
        negative["steps"][0]["delaySeconds"] = json!(-0.25);
        assert!(general_row(&negative).unwrap_err().contains("delaySeconds"));
    }

    #[test]
    fn look_at_fixture_operand_is_a_character_referent() {
        let row = json!({"talkId": 9, "lua": "x", "form": 2, "siteGroupId": 1,
            "termId": 1, "conditionGroupId": 1, "fixtureIds": [837], "unitIds": [1],
            "pairs": [[837, 1]], "tweet": {"id": 1},
            "steps": [{"op": "look_at_fixture", "fixture": 1}]});
        assert!(fixture_row(&row).is_ok());
    }
}
