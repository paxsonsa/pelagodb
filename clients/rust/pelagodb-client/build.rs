fn main() {
    let proto = "../../../proto/pelago.proto";
    tonic_build::configure()
        .build_server(false)
        .compile_protos(&[proto], &["../../../proto"])
        .expect("failed to compile proto");
}
