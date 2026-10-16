//! Source-resolved weather factory metadata. There are no runtime guessed defaults.
//! A profile crossfade and an effect's post-Stop destruction deadline are unrelated.
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "RawLifecycle")]
pub struct WeatherEffectLifecycle {
    time_until_destroy: f64,
    delay_source: DelaySource,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DelaySource {
    SerializedRoot,
    AddedComponentConstructor,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLifecycle {
    stop_behavior: String,
    time_until_destroy: f64,
    delay_source: DelaySource,
}

impl TryFrom<RawLifecycle> for WeatherEffectLifecycle {
    type Error = String;
    fn try_from(raw: RawLifecycle) -> Result<Self, Self::Error> {
        if raw.stop_behavior != "stopEmitting" {
            return Err(format!("unsupported source stop behavior: {}", raw.stop_behavior));
        }
        if !raw.time_until_destroy.is_finite() || raw.time_until_destroy < 0.0 {
            return Err("weather effect destruction delay must be finite and non-negative".into());
        }
        Ok(Self { time_until_destroy: raw.time_until_destroy, delay_source: raw.delay_source })
    }
}

impl WeatherEffectLifecycle {
    pub fn from_effect(effect: &serde_json::Value) -> Result<Self, String> {
        let value = effect.get("lifecycle").ok_or_else(||
            "weather effect lacks source lifecycle metadata; re-export with moly-root".to_owned())?;
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
    pub fn time_until_destroy(self) -> f64 { self.time_until_destroy }
    pub fn delay_source(self) -> DelaySource { self.delay_source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn consumes_explicit_serialized_or_factory_delay_without_a_weather_id_table() {
        for source in ["serializedRoot", "addedComponentConstructor"] {
            for delay in [0.0, 2.0, 3.75] {
                let effect = json!({"lifecycle":{"stopBehavior":"stopEmitting", "timeUntilDestroy":delay, "delaySource":source}});
                assert_eq!(WeatherEffectLifecycle::from_effect(&effect).unwrap().time_until_destroy(), delay);
            }
        }
    }

    #[test]
    fn absent_invalid_and_unsupported_metadata_fail_closed() {
        for effect in [json!({}), json!({"lifecycle":null}),
            json!({"lifecycle":{"stopBehavior":"stopEmitting", "timeUntilDestroy":-1, "delaySource":"serializedRoot"}}),
            json!({"lifecycle":{"stopBehavior":"stopEmittingAndClear", "timeUntilDestroy":2, "delaySource":"serializedRoot"}}),
            json!({"lifecycle":{"stopBehavior":"stopEmitting", "timeUntilDestroy":2, "delaySource":"guessed"}})] {
            assert!(WeatherEffectLifecycle::from_effect(&effect).is_err());
        }
    }
}
