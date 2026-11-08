//! WGSL module assembly.
//!
//! WGSL has no `import`, so every pipeline's module has to be self-contained. The C++ side gets
//! away with `#include "common.glsl"` in the preprocessor; here the loader textually concatenates
//! [`COMMON`] with one per-stage source and hands the result to `wgpu::Device::create_shader_module`
//! as a single source string.
//!
//! Every source is embedded with [`include_str!`] rather than read at runtime: the viewer then works
//! from a single binary, and a shader typo becomes a compile-time-visible path in the panic message
//! instead of a missing file at startup.

/// Shared structs, culling helpers, visibility-buffer packing and the PBR BRDF helpers.
/// Prepended to every module, exactly like `common.glsl` in `shaders/glsl/pbrtexture/`.
pub const COMMON: &str = include_str!("../shaders/common.wgsl");

/// One module per pipeline: `common.wgsl` + the stage source. Fourteen entries: fifteen WGSL files
/// exist, but `common.wgsl` is the shared prelude and is never a module of its own.
pub const MODULES: [(&str, &str); 14] = [
    ("bvhtraversal", include_str!("../shaders/bvhtraversal.wgsl")),
    ("error", include_str!("../shaders/error.wgsl")),
    ("culling", include_str!("../shaders/culling.wgsl")),
    ("swrasterize", include_str!("../shaders/swrasterize.wgsl")),
    ("clearimage", include_str!("../shaders/clearimage.wgsl")),
    ("merger", include_str!("../shaders/merger.wgsl")),
    ("hwrasterize", include_str!("../shaders/hwrasterize.wgsl")),
    ("shading", include_str!("../shaders/shading.wgsl")),
    ("skybox", include_str!("../shaders/skybox.wgsl")),
    ("depthcopy", include_str!("../shaders/depthcopy.wgsl")),
    ("genhiz", include_str!("../shaders/genhiz.wgsl")),
    ("genbrdflut", include_str!("../shaders/genbrdflut.wgsl")),
    ("irradiancecube", include_str!("../shaders/irradiancecube.wgsl")),
    ("prefilterenvmap", include_str!("../shaders/prefilterenvmap.wgsl")),
];

/// The concatenated source of one module, panicking with the module name if a source is missing.
pub fn source(name: &str) -> String {
    let stage = MODULES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("unknown shader module {name:?}"))
        .1;
    let mut src = String::with_capacity(COMMON.len() + stage.len() + 1);
    src.push_str(COMMON);
    src.push('\n');
    src.push_str(stage);
    src
}

/// Create every pipeline module up front. Doing this once at startup means a WGSL error is
/// reported before any frame is rendered, and `wgpu`'s own span information points into the
/// concatenated source above.
pub fn create_modules(device: &wgpu::Device) -> Vec<wgpu::ShaderModule> {
    MODULES
        .iter()
        .map(|(name, stage)| {
            let mut src = String::with_capacity(COMMON.len() + stage.len() + 1);
            src.push_str(COMMON);
            src.push('\n');
            src.push_str(stage);
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            })
        })
        .collect()
}

/// Index into [`MODULES`], so the renderer can name a pipeline without repeating a string.
pub const BVH_TRAVERSAL: usize = 0;
pub const ERROR: usize = 1;
pub const CULLING: usize = 2;
pub const SWRASTERIZE: usize = 3;
pub const CLEAR_IMAGE: usize = 4;
pub const MERGER: usize = 5;
pub const HWRASTERIZE: usize = 6;
pub const SHADING: usize = 7;
pub const SKYBOX: usize = 8;
pub const DEPTH_COPY: usize = 9;
pub const GEN_HIZ: usize = 10;
pub const GEN_BRDF_LUT: usize = 11;
pub const IRRADIANCE_CUBE: usize = 12;
pub const PREFILTER_ENV_MAP: usize = 13;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_declares_its_entry_points() {
        // A module that fails to declare the entry point the renderer asks for only shows up as a
        // wgpu label error at draw time, so pin the names here instead.
        // (module, entry points, uses `var<immediate>`)
        let expects: &[(&str, &[&str], bool)] = &[
            ("bvhtraversal", &["fn main"], true),
            ("error", &["fn main"], true),
            ("culling", &["fn main"], true),
            ("swrasterize", &["fn main"], false),
            ("clearimage", &["fn main"], false),
            ("merger", &["fn main"], true),
            ("hwrasterize", &["fn vs_main", "fn fs_main"], false),
            ("shading", &["fn vs_main", "fn fs_main"], true),
            ("skybox", &["fn vs_main", "fn fs_main"], false),
            ("depthcopy", &["fn main"], false),
            ("genhiz", &["fn main"], false),
            ("genbrdflut", &["fn vs_main", "fn fs_main"], false),
            ("irradiancecube", &["fn vs_main", "fn fs_main"], true),
            ("prefilterenvmap", &["fn vs_main", "fn fs_main"], true),
        ];
        for (name, fns, uses_immediates) in expects {
            let src = source(name);
            for f in fns.iter() {
                assert!(src.contains(f), "{name} is missing {f}");
            }
            assert_eq!(
                code_only(&src).contains("var<immediate>"),
                *uses_immediates,
                "{name} immediate usage mismatch"
            );
        }
    }

    /// Drop `//` line comments, so a comment that *mentions* a construct cannot satisfy a check
    /// for the construct itself.
    fn code_only(src: &str) -> String {
        src.lines()
            .map(|line| match line.find("//") {
                Some(at) => &line[..at],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn common_is_included_exactly_once_per_module() {
        let src = source("shading");
        assert_eq!(src.matches("fn packVis(").count(), 1);
        assert_eq!(src.matches("struct ClusterInfo").count(), 1);
        assert!(src.len() > COMMON.len());
    }

    #[test]
    fn no_glsl_only_builtins_survive() {
        // Recurring porting traps: these names exist in GLSL but not in WGSL, and naga reports them
        // as unknown identifiers deep inside a module.
        for (name, src) in MODULES {
            for banned in [
                "textureStorageLoad",
                "textureStorageDimensions",
                "fmod(",
                "textureStorageStore",
                "#version",
                "#include",
                "float(",
                "int(",
                "uint(",
            ] {
                assert!(!src.contains(banned), "{name} uses the GLSL-only {banned}");
            }
        }
    }

    /// The capabilities the renderer asks the adapter for, mirrored into naga's validator so this
    /// test checks the same program wgpu compiles. Without them the validator rejects the u64
    /// visibility buffer outright.
    const SHADER_CAPABILITIES: wgpu::naga::valid::Capabilities = {
        use wgpu::naga::valid::Capabilities as C;
        C::IMMEDIATES
            .union(C::SHADER_INT64)
            .union(C::SHADER_INT64_ATOMIC_MIN_MAX)
            .union(C::TEXTURE_INT64_ATOMIC)
            .union(C::TEXTURE_ATOMIC)
    };

    /// Parse *and* validate every module. `wgpu` re-exports naga, so this runs the exact front end
    /// and validator wgpu itself uses — and it runs without a GPU or a display, which is the only
    /// way the shader layer can be regression tested from `cargo test`.
    ///
    /// Parsing alone is not enough: naga accepts `u64 & 0x7FFFu` syntactically and only rejects it
    /// as a type error during validation, which on the GPU shows up as a panic inside
    /// `Device::create_shader_module` at start-up.
    #[test]
    fn every_module_is_valid_wgsl() {
        let mut validator = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            SHADER_CAPABILITIES,
        );
        for (name, _) in MODULES {
            let module = match wgpu::naga::front::wgsl::parse_str(&source(name)) {
                Ok(module) => module,
                Err(err) => panic!("{name}.wgsl: {err}"),
            };
            if let Err(err) = validator.validate(&module) {
                // `emit_to_string` resolves naga's spans against the concatenated source, which is
                // the only readable form of a validation error.
                panic!("{name}.wgsl: {}", err.emit_to_string(&source(name)));
            }
        }
        // `common.wgsl` has no entry point of its own, but a typo there breaks all 14 modules,
        // so it is parsed on its own too.
        if let Err(err) = wgpu::naga::front::wgsl::parse_str(COMMON) {
            panic!("common.wgsl: {err}");
        }
    }
}
