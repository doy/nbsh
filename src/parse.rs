pub mod ast;

#[derive(Debug)]
pub struct Pipeline {
    exes: Vec<Exe>,
}

impl Pipeline {
    pub fn into_exes(self) -> impl Iterator<Item = Exe> {
        self.exes.into_iter()
    }
}

#[derive(Debug)]
pub struct Exe {
    exe: std::path::PathBuf,
    args: Vec<String>,
    redirects: Vec<Redirect>,
}

impl Exe {
    pub fn exe(&self) -> &std::path::Path {
        &self.exe
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn redirects(&self) -> &[Redirect] {
        &self.redirects
    }

    pub fn shift(&mut self) {
        self.exe = std::path::PathBuf::from(self.args.remove(0));
    }
}

#[derive(Debug, Clone)]
pub struct Redirect {
    pub from: std::os::unix::io::RawFd,
    pub to: RedirectTarget,
    pub dir: Direction,
}

#[derive(Debug, Clone)]
pub enum RedirectTarget {
    Fd(std::os::unix::io::RawFd),
    File(std::path::PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    Append,
}

impl Direction {
    pub fn open(
        self,
        path: &std::path::Path,
    ) -> nix::Result<std::os::unix::io::RawFd> {
        use nix::fcntl::OFlag;
        use nix::sys::stat::Mode;
        Ok(match self {
            Self::In => nix::fcntl::open(
                path,
                OFlag::O_NOCTTY | OFlag::O_RDONLY,
                Mode::empty(),
            )?,
            Self::Out => nix::fcntl::open(
                path,
                OFlag::O_CREAT
                    | OFlag::O_NOCTTY
                    | OFlag::O_WRONLY
                    | OFlag::O_TRUNC,
                Mode::S_IRUSR
                    | Mode::S_IWUSR
                    | Mode::S_IRGRP
                    | Mode::S_IWGRP
                    | Mode::S_IROTH
                    | Mode::S_IWOTH,
            )?,
            Self::Append => nix::fcntl::open(
                path,
                OFlag::O_APPEND
                    | OFlag::O_CREAT
                    | OFlag::O_NOCTTY
                    | OFlag::O_WRONLY,
                Mode::S_IRUSR
                    | Mode::S_IWUSR
                    | Mode::S_IRGRP
                    | Mode::S_IWGRP
                    | Mode::S_IROTH
                    | Mode::S_IWOTH,
            )?,
        })
    }
}

#[derive(Debug)]
pub struct Error {
    input: String,
    e: anyhow::Error,
}

impl Error {
    fn new(input: &str, e: anyhow::Error) -> Self {
        Self {
            input: input.to_string(),
            e,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to parse {}: {}", self.input, self.e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.e)
    }
}
