use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type AspectRatioResolutionMap = BTreeMap<String, BTreeMap<String, ImageResolutionPreset>>;
pub type VideoResolutionConstraintMap = BTreeMap<String, VideoResolutionConstraint>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageResolutionPreset {
    pub size: String,
    pub token_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoResolutionConstraint {
    pub duration_sec: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelResolutionCatalog {
    pub image: BTreeMap<String, Vec<String>>,
    pub video: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub image_aspect_resolution_map: BTreeMap<String, AspectRatioResolutionMap>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub video_resolution_constraints: BTreeMap<String, VideoResolutionConstraintMap>,
}

/// Return UI-facing resolution presets grouped by asset kind and model key.
/// Resolution labels are intentionally case-sensitive (`1K/2K/4K`, never lowercase `k`).
pub fn model_resolution_catalog() -> ModelResolutionCatalog {
    let mut image = BTreeMap::new();
    let mut image_aspect_resolution_map = BTreeMap::new();

    for key in [
        "openai/gpt-image-1",
        "openai/gpt-image-1.5",
        "openai/gpt-image-1-mini",
    ] {
        image.insert(key.to_string(), default_openai_image_resolutions());
    }

    insert_image_model_with_aliases(
        &mut image,
        &mut image_aspect_resolution_map,
        &[
            "google/gemini-3.1-flash-image-preview",
            "nanobanana2",
            "nano-banana-2",
            "nano_banana_2",
        ],
        &["512", "1K", "2K", "4K"],
        nano_banana_2_table(),
    );

    insert_image_model_with_aliases(
        &mut image,
        &mut image_aspect_resolution_map,
        &[
            "google/gemini-3-pro-image-preview",
            "nanobanana_pro",
            "nano-banana-pro",
            "nano_banana_pro",
        ],
        &["1K", "2K", "4K"],
        nano_banana_pro_table(),
    );

    insert_image_model_with_aliases(
        &mut image,
        &mut image_aspect_resolution_map,
        &[
            "google/gemini-2.5-flash-image",
            "google/nanobanana",
            "nanobanana",
            "nano-banana",
            "nano_banana",
        ],
        &["1K"],
        nano_banana_table(),
    );

    let mut video = BTreeMap::new();
    let mut video_resolution_constraints = BTreeMap::new();

    let veo_resolutions = vec!["720p".to_string(), "1080p".to_string(), "4K".to_string()];
    let veo_constraints = veo_3_1_constraints();
    for key in [
        "google/veo_3_1",
        "google/veo-3.1-generate-preview",
        "veo_3_1",
        "veo3.1",
    ] {
        video.insert(key.to_string(), veo_resolutions.clone());
        video_resolution_constraints.insert(key.to_string(), veo_constraints.clone());
    }

    ModelResolutionCatalog {
        image,
        video,
        image_aspect_resolution_map,
        video_resolution_constraints,
    }
}

pub fn model_resolution_catalog_json() -> Value {
    serde_json::to_value(model_resolution_catalog())
        .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
}

fn default_openai_image_resolutions() -> Vec<String> {
    vec![
        "1024x1024".to_string(),
        "1536x1024".to_string(),
        "1024x1536".to_string(),
    ]
}

fn insert_image_model_with_aliases(
    image: &mut BTreeMap<String, Vec<String>>,
    image_aspect_resolution_map: &mut BTreeMap<String, AspectRatioResolutionMap>,
    keys: &[&str],
    labels: &[&str],
    aspect_table: AspectRatioResolutionMap,
) {
    let labels = labels
        .iter()
        .map(|label| (*label).to_string())
        .collect::<Vec<_>>();
    for key in keys {
        image.insert((*key).to_string(), labels.clone());
        image_aspect_resolution_map.insert((*key).to_string(), aspect_table.clone());
    }
}

fn preset(size: &str, token_count: u32) -> ImageResolutionPreset {
    ImageResolutionPreset {
        size: size.to_string(),
        token_count,
    }
}

fn tiers(entries: &[(&str, &str, u32)]) -> BTreeMap<String, ImageResolutionPreset> {
    let mut out = BTreeMap::new();
    for (tier, size, token_count) in entries {
        out.insert((*tier).to_string(), preset(size, *token_count));
    }
    out
}

fn nano_banana_2_table() -> AspectRatioResolutionMap {
    let mut out = BTreeMap::new();
    out.insert(
        "1:1".to_string(),
        tiers(&[
            ("512", "512x512", 747),
            ("1K", "1024x1024", 1120),
            ("2K", "2048x2048", 1120),
            ("4K", "4096x4096", 2000),
        ]),
    );
    out.insert(
        "1:4".to_string(),
        tiers(&[
            ("512", "256x1024", 747),
            ("1K", "512x2048", 1120),
            ("2K", "1024x4096", 1120),
            ("4K", "2048x8192", 2000),
        ]),
    );
    out.insert(
        "1:8".to_string(),
        tiers(&[
            ("512", "192x1536", 747),
            ("1K", "384x3072", 1120),
            ("2K", "768x6144", 1120),
            ("4K", "1536x12288", 2000),
        ]),
    );
    out.insert(
        "2:3".to_string(),
        tiers(&[
            ("512", "424x632", 747),
            ("1K", "848x1264", 1120),
            ("2K", "1696x2528", 1120),
            ("4K", "3392x5056", 2000),
        ]),
    );
    out.insert(
        "3:2".to_string(),
        tiers(&[
            ("512", "632x424", 747),
            ("1K", "1264x848", 1120),
            ("2K", "2528x1696", 1120),
            ("4K", "5056x3392", 2000),
        ]),
    );
    out.insert(
        "3:4".to_string(),
        tiers(&[
            ("512", "448x600", 747),
            ("1K", "896x1200", 1120),
            ("2K", "1792x2400", 1120),
            ("4K", "3584x4800", 2000),
        ]),
    );
    out.insert(
        "4:1".to_string(),
        tiers(&[
            ("512", "1024x256", 747),
            ("1K", "2048x512", 1120),
            ("2K", "4096x1024", 1120),
            ("4K", "8192x2048", 2000),
        ]),
    );
    out.insert(
        "4:3".to_string(),
        tiers(&[
            ("512", "600x448", 747),
            ("1K", "1200x896", 1120),
            ("2K", "2400x1792", 1120),
            ("4K", "4800x3584", 2000),
        ]),
    );
    out.insert(
        "4:5".to_string(),
        tiers(&[
            ("512", "464x576", 747),
            ("1K", "928x1152", 1120),
            ("2K", "1856x2304", 1120),
            ("4K", "3712x4608", 2000),
        ]),
    );
    out.insert(
        "5:4".to_string(),
        tiers(&[
            ("512", "576x464", 747),
            ("1K", "1152x928", 1120),
            ("2K", "2304x1856", 1120),
            ("4K", "4608x3712", 2000),
        ]),
    );
    out.insert(
        "8:1".to_string(),
        tiers(&[
            ("512", "1536x192", 747),
            ("1K", "3072x384", 1120),
            ("2K", "6144x768", 1120),
            ("4K", "12288x1536", 2000),
        ]),
    );
    out.insert(
        "9:16".to_string(),
        tiers(&[
            ("512", "384x688", 747),
            ("1K", "768x1376", 1120),
            ("2K", "1536x2752", 1120),
            ("4K", "3072x5504", 2000),
        ]),
    );
    out.insert(
        "16:9".to_string(),
        tiers(&[
            ("512", "688x384", 747),
            ("1K", "1376x768", 1120),
            ("2K", "2752x1536", 1120),
            ("4K", "5504x3072", 2000),
        ]),
    );
    out.insert(
        "21:9".to_string(),
        tiers(&[
            ("512", "792x168", 747),
            ("1K", "1584x672", 1120),
            ("2K", "3168x1344", 1120),
            ("4K", "6336x2688", 2000),
        ]),
    );
    out
}

fn nano_banana_pro_table() -> AspectRatioResolutionMap {
    let mut out = BTreeMap::new();
    out.insert(
        "1:1".to_string(),
        tiers(&[
            ("1K", "1024x1024", 1120),
            ("2K", "2048x2048", 1120),
            ("4K", "4096x4096", 2000),
        ]),
    );
    out.insert(
        "2:3".to_string(),
        tiers(&[
            ("1K", "848x1264", 1120),
            ("2K", "1696x2528", 1120),
            ("4K", "3392x5056", 2000),
        ]),
    );
    out.insert(
        "3:2".to_string(),
        tiers(&[
            ("1K", "1264x848", 1120),
            ("2K", "2528x1696", 1120),
            ("4K", "5056x3392", 2000),
        ]),
    );
    out.insert(
        "3:4".to_string(),
        tiers(&[
            ("1K", "896x1200", 1120),
            ("2K", "1792x2400", 1120),
            ("4K", "3584x4800", 2000),
        ]),
    );
    out.insert(
        "4:3".to_string(),
        tiers(&[
            ("1K", "1200x896", 1120),
            ("2K", "2400x1792", 1120),
            ("4K", "4800x3584", 2000),
        ]),
    );
    out.insert(
        "4:5".to_string(),
        tiers(&[
            ("1K", "928x1152", 1120),
            ("2K", "1856x2304", 1120),
            ("4K", "3712x4608", 2000),
        ]),
    );
    out.insert(
        "5:4".to_string(),
        tiers(&[
            ("1K", "1152x928", 1120),
            ("2K", "2304x1856", 1120),
            ("4K", "4608x3712", 2000),
        ]),
    );
    out.insert(
        "9:16".to_string(),
        tiers(&[
            ("1K", "768x1376", 1120),
            ("2K", "1536x2752", 1120),
            ("4K", "3072x5504", 2000),
        ]),
    );
    out.insert(
        "16:9".to_string(),
        tiers(&[
            ("1K", "1376x768", 1120),
            ("2K", "2752x1536", 1120),
            ("4K", "5504x3072", 2000),
        ]),
    );
    out.insert(
        "21:9".to_string(),
        tiers(&[
            ("1K", "1584x672", 1120),
            ("2K", "3168x1344", 1120),
            ("4K", "6336x2688", 2000),
        ]),
    );
    out
}

fn nano_banana_table() -> AspectRatioResolutionMap {
    let mut out = BTreeMap::new();
    out.insert("1:1".to_string(), tiers(&[("1K", "1024x1024", 1290)]));
    out.insert("2:3".to_string(), tiers(&[("1K", "832x1248", 1290)]));
    out.insert("3:2".to_string(), tiers(&[("1K", "1248x832", 1290)]));
    out.insert("3:4".to_string(), tiers(&[("1K", "864x1184", 1290)]));
    out.insert("4:3".to_string(), tiers(&[("1K", "1184x864", 1290)]));
    out.insert("4:5".to_string(), tiers(&[("1K", "896x1152", 1290)]));
    out.insert("5:4".to_string(), tiers(&[("1K", "1152x896", 1290)]));
    out.insert("9:16".to_string(), tiers(&[("1K", "768x1344", 1290)]));
    out.insert("16:9".to_string(), tiers(&[("1K", "1344x768", 1290)]));
    out.insert("21:9".to_string(), tiers(&[("1K", "1536x672", 1290)]));
    out
}

fn veo_3_1_constraints() -> VideoResolutionConstraintMap {
    let mut out = BTreeMap::new();
    out.insert(
        "720p".to_string(),
        VideoResolutionConstraint {
            duration_sec: vec![5, 6, 7, 8],
        },
    );
    out.insert(
        "1080p".to_string(),
        VideoResolutionConstraint {
            duration_sec: vec![8],
        },
    );
    out.insert(
        "4K".to_string(),
        VideoResolutionConstraint {
            duration_sec: vec![8],
        },
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_expected_veo_video_resolutions() {
        let catalog = model_resolution_catalog();
        let veo = catalog
            .video
            .get("google/veo_3_1")
            .expect("veo key must exist");
        assert_eq!(veo, &vec!["720p", "1080p", "4K"]);
        assert!(!veo.contains(&"4k".to_string()));
    }

    #[test]
    fn includes_expected_nanobanana_image_resolutions() {
        let catalog = model_resolution_catalog();
        let nb2 = catalog
            .image
            .get("google/gemini-3.1-flash-image-preview")
            .expect("nano banana 2 key must exist");
        assert_eq!(nb2, &vec!["512", "1K", "2K", "4K"]);
        assert!(!nb2.contains(&"1k".to_string()));
    }

    #[test]
    fn includes_nanobanana_ratio_mapping() {
        let catalog = model_resolution_catalog();
        let details = catalog
            .image_aspect_resolution_map
            .get("google/gemini-3.1-flash-image-preview")
            .expect("nano banana 2 detail table must exist");
        let ratio = details.get("16:9").expect("16:9 ratio must exist");
        let preset = ratio.get("4K").expect("4K preset must exist");
        assert_eq!(preset.size, "5504x3072");
        assert_eq!(preset.token_count, 2000);
    }

    #[test]
    fn json_shape_has_image_and_video_roots() {
        let value = model_resolution_catalog_json();
        assert!(value.get("image").is_some());
        assert!(value.get("video").is_some());
        assert!(value.get("image_aspect_resolution_map").is_some());
        assert!(value.get("video_resolution_constraints").is_some());
    }
}
