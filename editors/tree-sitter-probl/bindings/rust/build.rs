fn main() {
    let mut build = cc::Build::new();
    build.include("src").file("src/parser.c").file("src/scanner.c");
    build.std("c11");
    build.compile("tree-sitter-probl");
    for path in ["src/parser.c", "src/scanner.c", "src/tree_sitter/parser.h"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
