fn main() {
    // Chapter Freight — expose the build target triple so `pack install`
    // can refuse a bundle built for a foreign platform. `TARGET` is set
    // for build scripts but not for normal compilation, hence the relay.
    println!(
        "cargo:rustc-env=AIVYX_BUILD_TARGET={}",
        std::env::var("TARGET").expect("cargo sets TARGET for build scripts")
    );
}
