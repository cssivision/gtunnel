fn main() {
    tonic_build::compile_protos("proto/tunnel.proto").unwrap();
}