# MotionLoom Process Kernels (Seed Set)

This folder seeds one baseline WGSL helper kernel per process category (10 categories):

1. `transform_camera/transform_camera_affine.wgsl`
2. `color_tone/color_core.wgsl` (shared core for exposure/contrast/saturation/curves/LUT, vignette/film/BW, grain, glow/bloom)
3. `stylize_look/stylize_look_posterize.wgsl`
4. `blur_sharpen_detail/blur_sharpen_detail_gaussian.wgsl` (`mode`: `gaussian_5tap_h|gaussian_5tap_v|box|unsharp`)
5. `keying_matte_mask/keying_matte_mask_luma_key.wgsl`
6. `light_atmosphere/light_atmosphere_bloom_prefilter.wgsl`
7. `distortion_warp/distortion_warp_heat_haze.wgsl`
8. `composite/composite_core.wgsl` (`opacity`)
9. `transition/transition_core.wgsl`
10. `testing/effect_for_testing_run.wgsl` (testing-only)

Catalog API lives in `process_catalog.rs`.
