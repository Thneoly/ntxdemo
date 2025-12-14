fn main() {
    // 告诉 cargo 重新构建，如果 wit 文件改变
    println!("cargo:rerun-if-changed=wit/");
}
