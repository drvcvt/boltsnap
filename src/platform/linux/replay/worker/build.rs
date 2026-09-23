fn main() {
    #[cfg(feature = "native-cursor")]
    {
        let mut build = cc::Build::new();
        build
            .file("native/gpu.c")
            .flag_if_supported("-std=c11")
            .define("_GNU_SOURCE", None);
        for library in [
            "egl",
            "glesv2",
            "gbm",
            "vulkan",
            "libavutil",
            "libavfilter",
            "libavformat",
            "libavcodec",
        ] {
            let package = pkg_config::Config::new()
                .probe(library)
                .expect("native cursor GPU development dependency");
            for include in package.include_paths {
                build.include(include);
            }
        }
        build.compile("boltsnap_cursor_gpu");
        println!("cargo:rerun-if-changed=native/gpu.c");
        println!("cargo:rerun-if-changed=native/gpu.h");
    }
}
