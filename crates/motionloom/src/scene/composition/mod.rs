//! Scene compositing model.
//!
//! Owns layer/source routing, precompose instancing, blend, matte, mask
//! application, and effect-stack orchestration. Composition decides how
//! textures and draw results combine; it should not define primitive geometry.

mod effects;
mod raster;

pub(crate) use effects::{
    SceneMagnifyLensParams, SceneTextureOverlayParams, TextureOverlayKind, apply_box_blur_pass,
    apply_color_core_pass, apply_hsla_pass, apply_image_texture_overlay_pass, apply_layer_effects,
    apply_over_pass, apply_scene_filter_step, apply_scene_post_pass, build_scene_bloom_prefilter,
    composite_scene_bloom, is_color_key_alpha_effect, pass_param_expr, scene_post_bloom_params,
    scene_post_blur_passes, scene_post_brightness_amount, scene_post_glow_stack_params,
    scene_post_light_sweep_params, scene_post_magnify_lens_params,
    scene_post_texture_overlay_params, scene_post_tone_map_params,
};
pub(crate) use raster::{
    apply_alpha_mask, apply_alpha_mask_with_invert, apply_deform_grid, blend_pixel,
    blend_pixel_with_mode, composite_layer, composite_layer_affine, composite_layer_affine_blend,
    composite_layer_affine_blend_clipped, composite_layer_affine_clipped,
    composite_layer_projected_quad_blend_clipped, composite_transformed_layer,
    composite_transformed_layer_anchored, draw_rgba_image, shape_alpha_mask,
};
