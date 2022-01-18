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
// this isn't super relevant in a binary - if it's actually a problem, we'll
// just get a compilation failure
#![allow(clippy::future_not_send)]

mod env;
mod format;
mod info;
mod mutex;
mod parse;
mod prelude;
mod runner;
mod shell;

use prelude::*;

#[derive(structopt::StructOpt)]
#[structopt(about = "NoteBook SHell")]
struct Opt {
    #[structopt(short = "c")]
    command: Option<String>,

    #[structopt(long)]
    status_fd: Option<std::os::unix::io::RawFd>,
}

async fn async_main(opt: Opt) -> anyhow::Result<i32> {
    if let Some(command) = opt.command {
        let shell_write = opt.status_fd.and_then(|fd| {
            nix::sys::stat::fstat(fd).ok().map(|_| {
                // Safety: we don't create File instances for or read/write
                // data on this fd anywhere else
                unsafe { async_std::fs::File::from_raw_fd(fd) }
            })
        });

        return runner::run(&command, shell_write.as_ref()).await;
    }

    shell::main().await
}

#[paw::main]
fn main(opt: Opt) {
    match async_std::task::block_on(async_main(opt)) {
        Ok(code) => {
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("nbsh: {}", e);
            std::process::exit(1);
        }
    };
}
