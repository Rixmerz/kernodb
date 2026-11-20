//! Compiles `proto/kerno/v1/kerno.proto` into the gRPC server and client code `src/grpc.rs`
//! includes. `protox` is a protobuf compiler written in Rust, so the build needs no `protoc`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    let descriptors = protox::compile(["kerno/v1/kerno.proto"], ["proto"])?;
    tonic_prost_build::configure().compile_fds(descriptors)?;
    Ok(())
}
