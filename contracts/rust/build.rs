//! Compiles `contracts/proto/tomeandtile/v1/*.proto` without needing a `protoc`
//! binary: `protox` parses the files, `prost-build` generates the message
//! types and `pbjson-build` generates Protobuf-JSON serde impls.

use std::path::PathBuf;
use std::{env, fs};

use prost::Message;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = PathBuf::from("../proto");
    let proto_dir = proto_root.join("tomeandtile/v1");

    println!("cargo:rerun-if-changed={}", proto_dir.display());

    let mut files: Vec<PathBuf> = fs::read_dir(&proto_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "proto"))
        .collect();
    files.sort();

    let fds = protox::compile(&files, [&proto_root])?;
    let fds_bytes = fds.encode_to_vec();

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_fds(fds)?;

    pbjson_build::Builder::new()
        .out_dir(&out_dir)
        .register_descriptors(&fds_bytes)?
        .build(&[".tomeandtile.v1"])?;

    // pbjson-build does not escape enum values named `SELF` (prost renames the
    // Rust variant `Self` to `Self_`, pbjson still emits `Self`), which does not
    // compile. `TARGET_SHAPE_SELF` triggers this. Patch the generated serde file
    // until pbjson-build handles it; the replacements are no-ops once it does.
    let serde_path = out_dir.join("tomeandtile.v1.serde.rs");
    let serde_src = fs::read_to_string(&serde_path)?;
    let patched = serde_src
        .replace("::Self =>", "::Self_ =>")
        .replace("::Self)", "::Self_)");
    fs::write(&serde_path, patched)?;

    Ok(())
}
