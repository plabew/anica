use super::*;
#[test]
fn curve_drag_style_update_rewrites_only_target_pass_opacity() {
    let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="tmp" fmt="rgba16f" size={[1920,1080]} />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" effect="gaussian_5tap_h"
        in={["under"]} out={["tmp"]}
        params={{ sigma: "8.0" }} />
  <Pass id="fx_opacity" kind="compute" effect="opacity"
        in={["tmp"]} out={["out"]}
        params={{ opacity: curve("0.00:1.000:linear, 2.00:1.000:linear"), }} />
  <Present from="out" />
</Graph>
"#;

    // Simulate "point moved" result from curve UI.
    let points = vec![
        LayerFxCurvePoint {
            t_sec: 0.0,
            value: 0.137,
            ease: LayerFxCurveEase::EaseIn,
        },
        LayerFxCurvePoint {
            t_sec: 1.56,
            value: 0.929,
            ease: LayerFxCurveEase::Linear,
        },
        LayerFxCurvePoint {
            t_sec: 2.00,
            value: 0.700,
            ease: LayerFxCurveEase::EaseInOut,
        },
    ];
    let curve_expr = InspectorPanel::curve_points_to_expr(&points);
    let updated =
        InspectorPanel::upsert_pass_curve_param(script, "fx_opacity", "opacity", &curve_expr)
            .expect("should update target pass");

    assert!(
        updated.contains(
            "opacity: curve(\"0.00:0.137:ease_in, 1.56:0.929:linear, 2.00:0.700:ease_in_out\")"
        ),
        "updated script did not contain rewritten curve:\n{updated}"
    );
    assert!(
        updated.contains("<Pass id=\"fx_blur\""),
        "non-target pass was unexpectedly touched:\n{updated}"
    );
    assert!(
        !updated.contains("opacity: curve(\"0.00:1.000:linear, 2.00:1.000:linear\")"),
        "old curve value should be replaced:\n{updated}"
    );
}

#[test]
fn curve_drag_style_update_returns_none_for_unknown_pass() {
    let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_opacity" kind="compute" effect="opacity"
        in={["under"]} out={["out"]}
        params={{ opacity: "1.0" }} />
  <Present from="out" />
</Graph>
"#;

    let out = InspectorPanel::upsert_pass_curve_param(
        script,
        "missing_pass",
        "opacity",
        "curve(\"0.00:0.500:linear, 2.00:0.500:linear\")",
    );
    assert!(out.is_none(), "unknown pass id should not rewrite script");
}

#[test]
fn curve_drag_style_update_rewrites_sigma_for_blur_pass() {
    let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" effect="gaussian_5tap_h"
        in={["under"]} out={["out"]}
        params={{ sigma: "10.0" }} />
  <Present from="out" />
</Graph>
"#;

    let points = vec![
        LayerFxCurvePoint {
            t_sec: 0.0,
            value: 2.5,
            ease: LayerFxCurveEase::Linear,
        },
        LayerFxCurvePoint {
            t_sec: 1.5,
            value: 12.0,
            ease: LayerFxCurveEase::EaseInOut,
        },
    ];
    let curve_expr = InspectorPanel::curve_points_to_expr(&points);
    let updated = InspectorPanel::upsert_pass_curve_param(script, "fx_blur", "sigma", &curve_expr)
        .expect("should update blur sigma");

    assert!(
        updated.contains("sigma: curve(\"0.00:2.500:linear, 1.50:12.000:ease_in_out\")"),
        "sigma curve was not rewritten correctly:\n{updated}"
    );
}
