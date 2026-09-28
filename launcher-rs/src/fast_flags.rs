use crate::paths::Paths;
use serde_json::{Map, Value};
use std::fs;

pub struct FastFlags;

impl FastFlags {
    pub fn parse_flag_value(text: &str) -> Value {
        let s = text.trim();
        let lowered = s.to_lowercase();
        if lowered == "true" {
            Value::Bool(true)
        } else if lowered == "false" {
            Value::Bool(false)
        } else if let Ok(i) = s.parse::<i64>() {
            Value::Number(i.into())
        } else if let Ok(f) = s.parse::<f64>() {
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(s.to_string()))
        } else {
            Value::String(s.to_string())
        }
    }

    pub fn load(paths: &Paths) -> Map<String, Value> {
        let file = paths.fast_flags_file();
        if let Ok(content) = fs::read_to_string(&file) {
            if let Ok(Value::Object(map)) = serde_json::from_str(&content) {
                return map;
            }
        }
        Map::new()
    }

    pub fn save(paths: &Paths, flags: &Map<String, Value>) -> anyhow::Result<()> {
        let file = paths.fast_flags_file();
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        let json_str = serde_json::to_string_pretty(flags)?;
        fs::write(file, json_str)?;
        Ok(())
    }

    pub fn set_flag(paths: &Paths, name: &str, value: Value) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        flags.insert(name.to_string(), value);
        Self::save(paths, &flags)
    }

    pub fn remove_flag(paths: &Paths, name: &str) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        flags.remove(name);
        Self::save(paths, &flags)
    }

    pub fn set_fps_cap(paths: &Paths, fps: u32) -> anyhow::Result<()> {
        Self::set_flag(paths, "DFIntTaskSchedulerTargetFps", Value::from(fps))
    }

    pub fn apply_potato_mode(paths: &Paths, enable: bool) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        if enable {
            flags.insert("FIntRenderShadowIntensity".into(), serde_json::json!(0));
            flags.insert("FIntFRMMinGrassDistance".into(), serde_json::json!(0));
            flags.insert("FIntFRMMaxGrassDistance".into(), serde_json::json!(0));
            flags.insert("FFlagDisablePostFx".into(), serde_json::json!(true));
            flags.insert("DFFlagDisableBloom".into(), serde_json::json!(true));
            flags.insert("DFFlagDisableSunRays".into(), serde_json::json!(true));
            flags.insert("DFFlagDisableAtmosphere".into(), serde_json::json!(true));
            flags.insert("DFIntDebugFRMQualityLevelOverride".into(), serde_json::json!(1));
        } else {
            flags.remove("FFlagDisablePostFx");
            flags.remove("DFFlagDisableBloom");
            flags.remove("DFFlagDisableSunRays");
            flags.remove("DFFlagDisableAtmosphere");
            flags.remove("DFIntDebugFRMQualityLevelOverride");
            flags.insert("FIntRenderShadowIntensity".into(), serde_json::json!(1));
            flags.insert("FIntFRMMinGrassDistance".into(), serde_json::json!(100));
            flags.insert("FIntFRMMaxGrassDistance".into(), serde_json::json!(500));
        }
        Self::save(paths, &flags)
    }

    pub fn is_potato_mode(paths: &Paths) -> bool {
        let flags = Self::load(paths);
        flags.get("FFlagDisablePostFx")
            .map(|v| v.as_bool() == Some(true) || v.as_str() == Some("True") || v.as_str() == Some("true"))
            .unwrap_or(false)
    }

    pub fn apply_ultra_mode(paths: &Paths, enable: bool) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        if enable {
            flags.insert("DFIntDebugFRMQualityLevelOverride".into(), serde_json::json!(21));
            flags.insert("FIntDebugForceMSAASamples".into(), serde_json::json!(8));
            flags.insert("FIntFRMMinGrassDistance".into(), serde_json::json!(500));
            flags.insert("FIntFRMMaxGrassDistance".into(), serde_json::json!(1000));
            flags.insert("FIntRenderShadowIntensity".into(), serde_json::json!(100));
            flags.insert("DFFlagTextureQualityOverrideEnabled".into(), serde_json::json!(true));
            flags.insert("DFIntTextureQualityOverride".into(), serde_json::json!(3));
        } else {
            flags.remove("DFIntDebugFRMQualityLevelOverride");
            flags.remove("FIntDebugForceMSAASamples");
            flags.remove("DFFlagTextureQualityOverrideEnabled");
            flags.remove("DFIntTextureQualityOverride");
        }
        Self::save(paths, &flags)
    }

    pub fn is_ultra_mode(paths: &Paths) -> bool {
        let flags = Self::load(paths);
        flags.get("DFIntDebugFRMQualityLevelOverride")
            .map(|v| v.as_i64() == Some(21) || v.as_str() == Some("21"))
            .unwrap_or(false)
    }

    pub fn apply_disable_telemetry(paths: &Paths, enable: bool) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        if enable {
            flags.insert("FFlagDebugDisableTelemetry".into(), serde_json::json!(true));
            flags.insert("FFlagDisableCrashReporting".into(), serde_json::json!(true));
            flags.insert("DFFlagBrowserTrackerDisabled".into(), serde_json::json!(true));
        } else {
            flags.remove("FFlagDebugDisableTelemetry");
            flags.remove("FFlagDisableCrashReporting");
            flags.remove("DFFlagBrowserTrackerDisabled");
        }
        Self::save(paths, &flags)
    }

    pub fn is_telemetry_disabled(paths: &Paths) -> bool {
        let flags = Self::load(paths);
        flags.get("FFlagDebugDisableTelemetry")
            .map(|v| v.as_bool() == Some(true) || v.as_str() == Some("True") || v.as_str() == Some("true"))
            .unwrap_or(false)
    }
}
