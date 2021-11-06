use async_std::io::WriteExt as _;

async fn async_main() -> anyhow::Result<()> {
    async_std::io::stdout().write_all(b"hello world\n").await?;
    Ok(())
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
