use std::ffi::CStr;
use std::fmt::Debug;

use tree_sitter::{ffi::ts_node_string, InputEdit, Parser};

fn main() {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_djot::language())
        .expect("Error loading djot grammer");
    let source_code = "_This is regular_ not strong emphasis\n*strong*\n";
    let mut tree = parser.parse(source_code, None).unwrap();
    println!("{}", tree.root_node().to_sexp());
}
