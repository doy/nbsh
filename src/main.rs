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
}

async fn async_main(
    opt: Opt,
    shell_write: Option<&async_std::fs::File>,
) -> anyhow::Result<i32> {
    if let Some(command) = opt.command {
        return runner::run(&command, shell_write).await;
    }

    shell::main().await
}

#[paw::main]
fn main(opt: Opt) {
    // need to do this here because the async-std executor allocates some fds,
    // and so in the case where we aren't being called from the main shell and
    // fd 3 wasn't preallocated in advance, we need to be able to tell that
    // before async-std opens something on fd 3
    let shell_write = if nix::sys::stat::fstat(3).is_ok() {
        nix::fcntl::fcntl(
            3,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .unwrap();
        // Safety: we don't create File instances for or read/write data on fd
        // 3 anywhere else
        Some(unsafe { async_std::fs::File::from_raw_fd(3) })
    } else {
        None
    };

    match async_std::task::block_on(async_main(opt, shell_write.as_ref())) {
        Ok(code) => {
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("nbsh: {}", e);
            std::process::exit(1);
        }
    };
}
