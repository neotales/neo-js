//! Compose the checked-in TypeScript declarations for an explicit selection.
//!
//! Run from the checkout root:
//!
//! ```sh
//! cargo run --offline -p neojs --example compose-types -- --with os --with sqlite \
//!   --check-cli-manifest --check types/neo-js.d.ts
//! ```
//!
//! The example uses the same `declarations` module as the composition tests.

use std::io::{self, Write};

#[allow(dead_code)] // The example uses `run`; the tests use the other helpers.
#[path = "../src/declarations.rs"]
mod declarations;

fn main() {
    match declarations::run(std::env::args().skip(1)) {
        Ok(Some(text)) => {
            let mut stdout = io::stdout().lock();
            if let Err(error) = stdout
                .write_all(text.as_bytes())
                .and_then(|()| stdout.flush())
            {
                if error.kind() != io::ErrorKind::BrokenPipe {
                    eprintln!("compose-types: cannot write stdout: {error}");
                    std::process::exit(1);
                }
            }
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("compose-types: {error}");
            std::process::exit(1);
        }
    }
}
