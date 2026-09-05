#![deny(clippy::all)]

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "test-support")
))]
use napi::bindgen_prelude::*;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use napi_derive::napi;

#[cfg(target_os = "macos")]
mod app_menu;
mod automation;
mod color;
mod custom_elements;
mod diff;
mod element_tree;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod lua_app;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod lua_runtime;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod luax;
mod markdown;
mod motion;
mod renderer;
// The data model is public so `examples/bench_serde.rs` measures the real
// types instead of a copy that silently drifts from them.
pub mod retained_tree;
pub mod style;
mod syntax;
mod text;
mod theme;

#[cfg(all(
    feature = "test-support",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
mod test_renderer;

pub use element_tree::*;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use lua_app::*;
pub use renderer::*;
pub use style::*;

#[cfg(any(test, not(feature = "test-support")))]
const TEST_GPUIX_RENDERER_UNAVAILABLE: &str =
    "TestGpuixRenderer requires a native build with the test-support feature.";

/// True only when this binary compiled the real GPU test renderer.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[napi]
pub fn has_test_gpuix_renderer() -> bool {
    cfg!(feature = "test-support")
}

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "test-support")
))]
#[napi]
pub struct TestGpuixRenderer;

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "test-support")
))]
#[napi]
impl TestGpuixRenderer {
    #[napi(constructor)]
    pub fn new(_width: Option<f64>, _height: Option<f64>) -> Result<Self> {
        Err(Error::from_reason(TEST_GPUIX_RENDERER_UNAVAILABLE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_message_explains_required_feature() {
        assert!(TEST_GPUIX_RENDERER_UNAVAILABLE.contains("test-support feature"));
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[test]
    fn has_test_gpuix_renderer_matches_real_impl() {
        assert_eq!(has_test_gpuix_renderer(), cfg!(feature = "test-support"));
    }

    #[cfg(all(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        not(feature = "test-support")
    ))]
    #[test]
    fn stub_constructor_explains_why() {
        match TestGpuixRenderer::new(None, None) {
            Ok(_) => panic!("stub constructor must fail"),
            Err(err) => assert!(
                err.to_string().contains(TEST_GPUIX_RENDERER_UNAVAILABLE),
                "{err}"
            ),
        }
    }
}
