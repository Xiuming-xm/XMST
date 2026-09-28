use std::path::PathBuf;

fn main() {
    // x86_64-pc-windows-gnu 工具链下 embed-resource 需要 windres（GNU binutils）。
    // MinGW 未加入系统 PATH 时会导致嵌入静默降级为仅 manifest，这里显式注入。
    let path_has_mingw = std::env::var_os("PATH").map_or(false, |p| {
        std::env::split_paths(&p)
            .any(|d| d.to_string_lossy().to_lowercase().contains("mingw64"))
    });
    if !path_has_mingw {
        let mingw_bin = r"C:\Users\xiumi\dev\tools\mingw64\bin";
        let mut paths: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
        paths.insert(0, PathBuf::from(mingw_bin));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    }

    println!("cargo:rerun-if-changed=resource.rc");
    println!("cargo:rerun-if-changed=assets/xmst.ico");
    embed_resource::compile("resource.rc", embed_resource::NONE);

    // Spark 输出文件解析：编译官方 proto（spark_sampler.proto import "spark/spark.proto"）
    // prost-build 需要 protoc 二进制：优先系统 PROTOC 环境变量，缺失时使用 vendored protoc
    if std::env::var_os("PROTOC").is_none() {
        if let Ok(p) = protoc_bin_vendored::protoc_bin_path() {
            std::env::set_var("PROTOC", p);
        }
    }
    println!("cargo:rerun-if-changed=src/spark_proto/spark/spark.proto");
    println!("cargo:rerun-if-changed=src/spark_proto/spark/spark_sampler.proto");
    let mut config = prost_build::Config::new();
    config
        .compile_protos(&["src/spark_proto/spark/spark_sampler.proto"], &["src/spark_proto/"])
        .expect("编译 spark protos 失败");
}
