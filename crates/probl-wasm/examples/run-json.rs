//! Native form of the playground adapter, for native/WASM transport parity.
use std::io::{self, Read};

fn main() {
    let mut request = String::new();
    io::stdin().read_to_string(&mut request).unwrap();
    println!("{}", probl_wasm::run(&request, &mut |_| {}, None));
}
