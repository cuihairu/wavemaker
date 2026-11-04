//! Build script: compiles the proto/ tree into Rust types.
//!
//! Run from the workspace root; `protoc` must be on PATH (CI installs
//! protobuf-compiler).

fn main() {
    let proto_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../proto");

    let protos = [
        "common/v1/common_types.proto",
        "control/v1/control_service.proto",
        "backend/v1/backend_service.proto",
        "engine/v1/engine_bridge.proto",
        "metrics/v1/metrics_events.proto",
    ]
    .map(|p| format!("{proto_root}/{p}"));

    tonic_prost_build::configure()
        .compile_protos(&protos, &[proto_root.to_string()])
        .expect("failed to compile wavemaker protobuf contracts");

    println!("cargo:rerun-if-changed={proto_root}");
}
