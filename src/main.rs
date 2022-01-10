// will uncomment this once it is closer to release
// #![warn(clippy::cargo)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::as_conversions)]
#![warn(clippy::get_unwrap)]
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::similar_names)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::type_complexity)]

mod env;
mod format;
mod info;
mod mutex;
mod parse;
mod prelude;
mod runner;
mod shell;

async fn async_main() -> anyhow::Result<i32> {
    if std::env::args().nth(1).as_deref() == Some("--internal-cmd-runner") {
        return runner::main().await;
    }

    shell::main().await
}

fn main() {
    match async_std::task::block_on(async_main()) {
        Ok(code) => {
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("nbsh: {}", e);
            std::process::exit(1);
        }
    };
}
