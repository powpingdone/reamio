fn main() {
    println!("cargo:rerun-if-changed=server/src/migrations");
}
