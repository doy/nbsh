#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::unused_self)]

mod history;
mod nbsh;
mod repl;

async fn async_main() -> anyhow::Result<()> {
    nbsh::run().await
}

fn main() {
    match async_std::task::block_on(async_main()) {
        Ok(_) => (),
        Err(e) => {
            eprintln!("nbsh: {}", e);
            std::process::exit(1);
        }
    };
}
