use std::collections::HashMap;
use std::sync::OnceLock;

use crate::dsl::PassNode;

pub fn normalize_effect_key(effect: &str) -> String {
    effect.trim().trim_matches('"').trim().to_ascii_lowercase()
}

pub fn normalize_kernel_name(kernel: &str) -> String {
    kernel.trim().trim_matches('"').trim().to_string()
}

pub fn default_kernel_for_effect(effect: &str) -> Option<&'static str> {
    static EFFECT_KERNEL_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    let map = EFFECT_KERNEL_MAP.get_or_init(load_effect_kernel_map);
    map.get(&normalize_effect_key(effect)).map(|v| v.as_str())
}

pub fn resolve_pass_kernel(pass: &PassNode) -> Option<String> {
    if let Some(kernel) = pass.kernel.as_deref() {
        let normalized = normalize_kernel_name(kernel);
        if !normalized.is_empty() {
            return Some(normalized);
        }
        return None;
    }
    default_kernel_for_effect(&pass.effect).map(|v| v.to_string())
}

fn load_effect_kernel_map() -> HashMap<String, String> {
    const MAP_TEXT: &str = include_str!("kernels/effect_kernel_map.kv");
    let mut out = HashMap::<String, String>::new();
    for raw_line in MAP_TEXT.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        let effect = normalize_effect_key(left);
        let kernel = normalize_kernel_name(right);
        if !effect.is_empty() && !kernel.is_empty() {
            out.insert(effect, kernel);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{default_kernel_for_effect, resolve_pass_kernel};
    use crate::dsl::{PassKind, PassNode, PassParam, ResourceRef};

    #[test]
    fn default_mapping_contains_transition_effects() {
        assert_eq!(
            default_kernel_for_effect("fade_in"),
            Some("transition_core.wgsl")
        );
        assert_eq!(
            default_kernel_for_effect("dissolve"),
            Some("transition_core.wgsl")
        );
    }

    #[test]
    fn resolve_prefers_explicit_kernel_when_present() {
        let pass = PassNode {
            id: "p1".to_string(),
            kind: PassKind::Compute,
            role: None,
            kernel: Some("my_custom.wgsl".to_string()),
            mode: None,
            effect: "gaussian_5tap_h".to_string(),
            transition: None,
            transition_fallback: None,
            transition_easing: None,
            transition_clips: None,
            inputs: vec![ResourceRef::Id {
                id: "src".to_string(),
            }],
            outputs: vec![ResourceRef::Id {
                id: "out".to_string(),
            }],
            params: vec![PassParam {
                key: "sigma".to_string(),
                value: "2.0".to_string(),
            }],
            iterate: None,
            pingpong: None,
            cache: None,
            blend: None,
            load_op: None,
            store_op: None,
        };
        assert_eq!(
            resolve_pass_kernel(&pass).as_deref(),
            Some("my_custom.wgsl")
        );
    }
}
