use crate::paths::Paths;
use serde_json::{Map, Value};
use std::fs;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightingTechnology {
    Default = 0,
    Voxel = 1,
    ShadowMap = 2,
    Future = 3,
}

#[allow(dead_code)]
impl LightingTechnology {
    pub fn from_u32(val: u32) -> Self {
        match val {
            1 => Self::Voxel,
            2 => Self::ShadowMap,
            3 => Self::Future,
            _ => Self::Default,
        }
    }

    pub fn as_u32(&self) -> u32 {
        *self as u32
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Default => "По умолчанию (выбор игры)",
            Self::Voxel => "Voxel (Фаза 1)",
            Self::ShadowMap => "ShadowMap (Фаза 2)",
            Self::Future => "Future is Bright (Фаза 3)",
        }
    }
}

pub struct FastFlags;

#[allow(dead_code)]
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

    pub fn clean_json_str(raw: &str) -> String {
        let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
        let mut out = String::with_capacity(raw.len());
        let mut in_string = false;
        let mut escaped = false;
        let chars: Vec<char> = raw.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];
            if in_string {
                out.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                i += 1;
            } else {
                if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    i += 2;
                    while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                        i += 1;
                    }
                } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                    i += 2;
                    while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                        i += 1;
                    }
                    if i + 1 < chars.len() {
                        i += 2;
                    } else {
                        i = chars.len();
                    }
                } else if c == '"' {
                    in_string = true;
                    out.push(c);
                    i += 1;
                } else {
                    out.push(c);
                    i += 1;
                }
            }
        }

        let mut final_out = String::with_capacity(out.len());
        let chars: Vec<char> = out.chars().collect();
        in_string = false;
        escaped = false;
        let mut idx = 0;

        while idx < chars.len() {
            let c = chars[idx];
            if in_string {
                final_out.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                idx += 1;
            } else if c == '"' {
                in_string = true;
                final_out.push(c);
                idx += 1;
            } else if c == ',' {
                let mut next_idx = idx + 1;
                while next_idx < chars.len() && chars[next_idx].is_whitespace() {
                    next_idx += 1;
                }
                if next_idx < chars.len() && (chars[next_idx] == '}' || chars[next_idx] == ']') {
                    idx += 1;
                } else {
                    final_out.push(c);
                    idx += 1;
                }
            } else {
                final_out.push(c);
                idx += 1;
            }
        }

        final_out
    }

    pub fn load(paths: &Paths) -> Map<String, Value> {
        let file = paths.fast_flags_file();
        if let Ok(content) = fs::read_to_string(&file) {
            let cleaned = Self::clean_json_str(&content);
            if let Ok(Value::Object(map)) = serde_json::from_str(&cleaned) {
                return map;
            }
        }
        Map::new()
    }

    pub fn save(paths: &Paths, flags: &Map<String, Value>) -> anyhow::Result<()> {
        let file = paths.fast_flags_file();
        let parent = file.parent().ok_or_else(|| anyhow::anyhow!("Invalid fast flags path"))?;
        fs::create_dir_all(parent)?;
        let json_str = serde_json::to_string_pretty(flags)?;
        let tmp_file = parent.join(format!(".ClientAppSettings.json.tmp.{}", std::process::id()));
        fs::write(&tmp_file, json_str)?;
        fs::rename(&tmp_file, &file)?;
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

    // --- Lighting technology (Future is Bright Phase 3 / ShadowMap / Voxel) ---
    pub fn apply_lighting_technology(paths: &Paths, tech: LightingTechnology) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        match tech {
            LightingTechnology::Default => {
                flags.remove("FFlagDebugForceFutureIsBrightPhase3");
                flags.remove("DFFlagDebugRenderForceTechnology");
            }
            LightingTechnology::Voxel => {
                flags.remove("FFlagDebugForceFutureIsBrightPhase3");
                flags.insert("DFFlagDebugRenderForceTechnology".into(), serde_json::json!(1));
            }
            LightingTechnology::ShadowMap => {
                flags.remove("FFlagDebugForceFutureIsBrightPhase3");
                flags.insert("DFFlagDebugRenderForceTechnology".into(), serde_json::json!(2));
            }
            LightingTechnology::Future => {
                flags.insert("FFlagDebugForceFutureIsBrightPhase3".into(), serde_json::json!(true));
                flags.insert("DFFlagDebugRenderForceTechnology".into(), serde_json::json!(3));
            }
        }
        Self::save(paths, &flags)
    }

    pub fn get_lighting_technology(paths: &Paths) -> LightingTechnology {
        let flags = Self::load(paths);
        if let Some(val) = flags.get("DFFlagDebugRenderForceTechnology") {
            let num = val.as_i64().or_else(|| val.as_str().and_then(|s| s.parse::<i64>().ok()));
            match num {
                Some(1) => return LightingTechnology::Voxel,
                Some(2) => return LightingTechnology::ShadowMap,
                Some(3) => return LightingTechnology::Future,
                _ => {}
            }
        }
        if let Some(val) = flags.get("FFlagDebugForceFutureIsBrightPhase3") {
            if val.as_bool() == Some(true) || val.as_str() == Some("True") || val.as_str() == Some("true") {
                return LightingTechnology::Future;
            }
        }
        LightingTechnology::Default
    }

    pub fn apply_future_is_bright(paths: &Paths, enable: bool) -> anyhow::Result<()> {
        Self::apply_lighting_technology(
            paths,
            if enable { LightingTechnology::Future } else { LightingTechnology::Default },
        )
    }

    pub fn is_future_is_bright(paths: &Paths) -> bool {
        Self::get_lighting_technology(paths) == LightingTechnology::Future
    }

    // --- Material styling (Classic 2021 pre-update materials) ---
    pub fn apply_classic_materials(paths: &Paths, enable: bool) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        if enable {
            flags.insert("FFlagFixGraphicsQuality".into(), serde_json::json!(true));
            flags.insert("DFFlagDisableNewMaterials2022".into(), serde_json::json!(true));
        } else {
            flags.remove("FFlagFixGraphicsQuality");
            flags.remove("DFFlagDisableNewMaterials2022");
        }
        Self::save(paths, &flags)
    }

    pub fn is_classic_materials(paths: &Paths) -> bool {
        let flags = Self::load(paths);
        let mat2022 = flags.get("DFFlagDisableNewMaterials2022")
            .map(|v| v.as_bool() == Some(true) || v.as_str() == Some("True") || v.as_str() == Some("true"))
            .unwrap_or(false);
        let fix_gfx = flags.get("FFlagFixGraphicsQuality")
            .map(|v| v.as_bool() == Some(true) || v.as_str() == Some("True") || v.as_str() == Some("true"))
            .unwrap_or(false);
        mat2022 || fix_gfx
    }

    // --- Texture resolution / detail (Disable low-res texture LOD degradation) ---
    pub fn apply_max_texture_detail(paths: &Paths, enable: bool) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        if enable {
            flags.insert("FIntRenderTextureDetail".into(), serde_json::json!(3));
            flags.insert("DFIntTextureQualityOverride".into(), serde_json::json!(3));
            flags.insert("DFFlagTextureQualityOverrideEnabled".into(), serde_json::json!(true));
        } else {
            flags.remove("FIntRenderTextureDetail");
            flags.remove("DFIntTextureQualityOverride");
            flags.remove("DFFlagTextureQualityOverrideEnabled");
        }
        Self::save(paths, &flags)
    }

    pub fn is_max_texture_detail(paths: &Paths) -> bool {
        let flags = Self::load(paths);
        let detail = flags.get("FIntRenderTextureDetail")
            .map(|v| v.as_i64() == Some(3) || v.as_str() == Some("3"))
            .unwrap_or(false);
        let quality = flags.get("DFIntTextureQualityOverride")
            .map(|v| v.as_i64() == Some(3) || v.as_str() == Some("3"))
            .unwrap_or(false);
        detail || quality
    }

    // --- UI: Toggle modern in-game Chrome UI ---
    pub fn apply_chrome_ui(paths: &Paths, enable: bool) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        if enable {
            flags.insert("FFlagEnableInGameMenuChrome".into(), serde_json::json!(true));
        } else {
            flags.remove("FFlagEnableInGameMenuChrome");
        }
        Self::save(paths, &flags)
    }

    pub fn is_chrome_ui_enabled(paths: &Paths) -> bool {
        let flags = Self::load(paths);
        flags.get("FFlagEnableInGameMenuChrome")
            .map(|v| v.as_bool() == Some(true) || v.as_str() == Some("True") || v.as_str() == Some("true"))
            .unwrap_or(false)
    }

    pub fn ensure_compatibility_flags(paths: &Paths) -> anyhow::Result<()> {
        let mut flags = Self::load(paths);
        let mut changed = false;

        let compat_flags = [
            ("FFlagCookieProtocolEnabled", serde_json::json!("False")),
            ("DFFlagCookieProtocolEnabled", serde_json::json!("False")),
            ("FFlagUnifiedCookieProtocolEnabled", serde_json::json!("False")),
            ("DFFlagUnifiedCookieProtocolEnabled", serde_json::json!("False")),
            ("FFlagUnifiedCookieProtocolEnabledSticky", serde_json::json!("False")),
            ("DFFlagUnifiedCookieProtocolEnabledSticky", serde_json::json!("False")),
            ("FFlagDebugDisableRbxTransportDummyClient", serde_json::json!("True")),
            ("DFFlagDebugDisableRbxTransportDummyClient", serde_json::json!("True")),
            ("FFlagDebugDisableRbxTransportDummyServer", serde_json::json!("True")),
            ("DFFlagDebugDisableRbxTransportDummyServer", serde_json::json!("True")),
        ];

        for (k, v) in compat_flags {
            if flags.get(k) != Some(&v) {
                flags.insert(k.to_string(), v);
                changed = true;
            }
        }

        // Remove flags that break OpenGL shaders (e.g. Unified light culling requires
        // compute shaders not present in macOS Core Profile 4.1) or cause map loading freezes.
        if flags.remove("FFlagFastGPULightCulling3").is_some() {
            changed = true;
        }
        if flags.remove("DFIntCSGLevelOfDetailSwitchingDistance").is_some() {
            changed = true;
        }
        if flags.remove("DFIntCSGLevelOfDetailSwitchingDistanceL2").is_some() {
            changed = true;
        }

        let perf_defaults = [
            ("FFlagPreloadAllFonts", serde_json::json!("True")),
            ("FFlagPreloadTextureItems", serde_json::json!("True")),
            ("DFIntTaskSchedulerTargetFps", serde_json::json!(240)),
            ("DFIntAssetPreloadMaxParallelTasks", serde_json::json!(64)),
            ("DFIntContentProviderPreloadTasks", serde_json::json!(32)),
            ("FIntRuntimeMaxNumOfThreads", serde_json::json!(8)),
            ("FFlagDebugDisableTelemetry", serde_json::json!("True")),
            ("FFlagDisableCrashReporting", serde_json::json!("True")),
            ("FFlagEnableInGameMenuControls", serde_json::json!("True")),
        ];

        for (k, v) in perf_defaults {
            if !flags.contains_key(k) {
                flags.insert(k.to_string(), v);
                changed = true;
            }
        }

        if changed {
            Self::save(paths, &flags)?;
        }
        Ok(())
    }

    // --- Helper methods to import and export JSON ---
    pub fn import_json(paths: &Paths, json_str: &str) -> anyhow::Result<usize> {
        let trimmed = json_str.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Строка JSON пуста");
        }
        let cleaned = Self::clean_json_str(trimmed);
        let parsed: Value = serde_json::from_str(&cleaned)
            .map_err(|e| anyhow::anyhow!("Неверный синтаксис JSON: {e}"))?;
        let map = match parsed {
            Value::Object(m) => m,
            _ => anyhow::bail!("JSON должен быть объектом вида {{\"ИмяФлага\": значение}}"),
        };
        let count = map.len();
        let mut flags = Self::load(paths);
        for (k, v) in map {
            let clean_k = k.trim().to_string();
            if !clean_k.is_empty() {
                flags.insert(clean_k, v);
            }
        }
        Self::save(paths, &flags)?;
        Ok(count)
    }

    pub fn export_json(paths: &Paths) -> anyhow::Result<String> {
        let flags = Self::load(paths);
        let s = serde_json::to_string_pretty(&flags)?;
        Ok(s)
    }

    pub fn import_flags_json(paths: &Paths, json_str: &str) -> anyhow::Result<usize> {
        Self::import_json(paths, json_str)
    }

    pub fn export_flags_json(paths: &Paths) -> anyhow::Result<String> {
        Self::export_json(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_paths() -> (TempDir, Paths) {
        let temp = TempDir::new().unwrap();
        let paths = Paths {
            project_dir: temp.path().to_path_buf(),
            data_dir: temp.path().to_path_buf(),
            config_dir: temp.path().join("config"),
            cache_dir: temp.path().to_path_buf(),
            darling_prefix: temp.path().join("darling"),
            darling_sysroot: temp.path().join("sysroot"),
        };
        (temp, paths)
    }

    #[test]
    fn test_lighting_presets() {
        let (_tmp, paths) = create_test_paths();
        assert_eq!(FastFlags::get_lighting_technology(&paths), LightingTechnology::Default);

        FastFlags::apply_lighting_technology(&paths, LightingTechnology::Voxel).unwrap();
        assert_eq!(FastFlags::get_lighting_technology(&paths), LightingTechnology::Voxel);

        FastFlags::apply_lighting_technology(&paths, LightingTechnology::ShadowMap).unwrap();
        assert_eq!(FastFlags::get_lighting_technology(&paths), LightingTechnology::ShadowMap);

        FastFlags::apply_lighting_technology(&paths, LightingTechnology::Future).unwrap();
        assert_eq!(FastFlags::get_lighting_technology(&paths), LightingTechnology::Future);
        assert!(FastFlags::is_future_is_bright(&paths));

        FastFlags::apply_lighting_technology(&paths, LightingTechnology::Default).unwrap();
        assert_eq!(FastFlags::get_lighting_technology(&paths), LightingTechnology::Default);
        assert!(!FastFlags::is_future_is_bright(&paths));
    }

    #[test]
    fn test_classic_materials() {
        let (_tmp, paths) = create_test_paths();
        assert!(!FastFlags::is_classic_materials(&paths));

        FastFlags::apply_classic_materials(&paths, true).unwrap();
        assert!(FastFlags::is_classic_materials(&paths));

        FastFlags::apply_classic_materials(&paths, false).unwrap();
        assert!(!FastFlags::is_classic_materials(&paths));
    }

    #[test]
    fn test_max_texture_detail() {
        let (_tmp, paths) = create_test_paths();
        assert!(!FastFlags::is_max_texture_detail(&paths));

        FastFlags::apply_max_texture_detail(&paths, true).unwrap();
        assert!(FastFlags::is_max_texture_detail(&paths));

        FastFlags::apply_max_texture_detail(&paths, false).unwrap();
        assert!(!FastFlags::is_max_texture_detail(&paths));
    }

    #[test]
    fn test_chrome_ui() {
        let (_tmp, paths) = create_test_paths();
        assert!(!FastFlags::is_chrome_ui_enabled(&paths));

        FastFlags::apply_chrome_ui(&paths, true).unwrap();
        assert!(FastFlags::is_chrome_ui_enabled(&paths));

        FastFlags::apply_chrome_ui(&paths, false).unwrap();
        assert!(!FastFlags::is_chrome_ui_enabled(&paths));
    }

    #[test]
    fn test_import_export_json() {
        let (_tmp, paths) = create_test_paths();
        let initial_json = r#"{
            "FFlagCustomTest": true,
            "DFIntTargetValue": 120
        }"#;

        let count = FastFlags::import_json(&paths, initial_json).unwrap();
        assert_eq!(count, 2);

        let exported = FastFlags::export_json(&paths).unwrap();
        assert!(exported.contains("FFlagCustomTest"));
        assert!(exported.contains("DFIntTargetValue"));

        // Test merge
        let second_json = r#"{
            "FFlagAnother": "hello"
        }"#;
        let count2 = FastFlags::import_json(&paths, second_json).unwrap();
        assert_eq!(count2, 1);

        let merged = FastFlags::load(&paths);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged.get("FFlagAnother").unwrap().as_str(), Some("hello"));
        assert_eq!(merged.get("FFlagCustomTest").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_clean_json_str() {
        let dirty = r#"{
            // Comments are removed
            "flag1": true,
            "flag2": "value, with, commas", // trailing line comment
            "flag3": 42,
        }"#;

        let cleaned = FastFlags::clean_json_str(dirty);
        let parsed: Value = serde_json::from_str(&cleaned).expect("Should parse cleaned JSON");
        assert_eq!(parsed["flag1"], true);
        assert_eq!(parsed["flag2"], "value, with, commas");
        assert_eq!(parsed["flag3"], 42);
    }

    #[test]
    fn test_ensure_compatibility_flags() {
        let (_tmp, paths) = create_test_paths();
        FastFlags::ensure_compatibility_flags(&paths).unwrap();
        let flags = FastFlags::load(&paths);
        assert_eq!(flags.get("FFlagCookieProtocolEnabled").and_then(|v| v.as_str()), Some("False"));
        assert_eq!(flags.get("DFFlagCookieProtocolEnabled").and_then(|v| v.as_str()), Some("False"));
        assert_eq!(flags.get("FFlagUnifiedCookieProtocolEnabled").and_then(|v| v.as_str()), Some("False"));
        assert_eq!(flags.get("DFFlagUnifiedCookieProtocolEnabled").and_then(|v| v.as_str()), Some("False"));
        assert!(flags.get("FFlagFastGPULightCulling3").is_none());
        assert_eq!(flags.get("DFIntAssetPreloadMaxParallelTasks").and_then(|v| v.as_i64()), Some(64));
        assert_eq!(flags.get("DFIntTaskSchedulerTargetFps").and_then(|v| v.as_i64()), Some(240));
    }

    #[test]
    fn test_clean_json_str_with_block_comments() {
        let dirty = "\u{FEFF}{\n/* block comment */\n\"flag\": true\n}";
        let cleaned = FastFlags::clean_json_str(dirty);
        let parsed: Value = serde_json::from_str(&cleaned).expect("Should parse");
        assert_eq!(parsed["flag"], true);
    }
}
