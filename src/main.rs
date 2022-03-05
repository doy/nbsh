// will uncomment this once it is closer to release
// #![warn(clippy::cargo)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::as_conversions)]
#![warn(clippy::get_unwrap)]
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::option_option)]
#![allow(clippy::similar_names)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::type_complexity)]
// this isn't super relevant in a binary - if it's actually a problem, we'll
// just get a compilation failure
#![allow(clippy::future_not_send)]

mod env;
mod format;
mod info;
mod parse;
mod prelude;
mod runner;
mod shell;

use prelude::*;

use clap::Parser as _;

#[derive(clap::Parser)]
#[clap(about = "NoteBook SHell")]
struct Opt {
    #[clap(short = 'c')]
    command: Option<String>,

    #[clap(long)]
    status_fd: Option<std::os::unix::io::RawFd>,
}

#[tokio::main]
async fn async_main(opt: Opt) -> Result<i32> {
    if let Some(command) = opt.command {
        let mut shell_write = opt.status_fd.and_then(|fd| {
            nix::sys::stat::fstat(fd).ok().map(|_| {
                // Safety: we don't create File instances for or read/write
                // data on this fd anywhere else
                unsafe { tokio::fs::File::from_raw_fd(fd) }
            })
        });

        return runner::main(command, &mut shell_write).await;
    }

    shell::main().await
}

fn main() {
    match async_main(Opt::parse()) {
        Ok(code) => {
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("nbsh: {}", e);
            std::process::exit(1);
        }
    };
}
