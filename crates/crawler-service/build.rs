fn main() {
    println!("cargo::rustc-check-cfg=cfg(crawler_standalone)");
    println!("cargo::rustc-cfg=crawler_standalone");
}
