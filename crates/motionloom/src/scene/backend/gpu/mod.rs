//! Scene GPU backend internals.

mod compositor;
pub(crate) mod shaders;

pub(crate) use compositor::WgpuSceneCompositor;
