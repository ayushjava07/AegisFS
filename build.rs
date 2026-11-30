// Build script for the gRPC surface: compiles the v1 protocol into typed
// tonic/prost stubs before the crate compiles.
//
// Generated code lands in OUT_DIR and is re-generated only when the proto or
// this script changes; the crate keeps no checked-in copies of the stubs.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/runvane/v1/api.proto"], &["proto"])?;
    Ok(())
}
