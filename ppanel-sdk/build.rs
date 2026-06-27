fn main() {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let proto_file = manifest_dir.join("../plugin.proto");
    let proto_dir = proto_file.parent().unwrap().to_path_buf();
    let google_include = std::path::PathBuf::from("/usr/include");
    println!("cargo:rerun-if-changed={}", proto_file.display());
    prost_build::Config::new()
        .compile_protos(&[proto_file], &[proto_dir, google_include])
        .expect("failed to compile proto files");
}
