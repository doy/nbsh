pub mod ast;

#[derive(Debug, Eq, PartialEq)]
pub struct Pipeline {
    exes: Vec<Exe>,
}

impl Pipeline {
    pub fn into_exes(self) -> impl Iterator<Item = Exe> {
        self.exes.into_iter()
    }
}

#[derive(Debug, Eq, PartialEq)]
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Redirect {
    pub from: std::os::unix::io::RawFd,
    pub to: RedirectTarget,
    pub dir: Direction,
}

#[derive(Debug, Clone, Eq, PartialEq)]
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

#[derive(Debug, Eq, PartialEq)]
pub struct Error {
    input: String,
    e: pest::error::Error<ast::Rule>,
}

impl Error {
    fn new(input: &str, e: pest::error::Error<ast::Rule>) -> Self {
        Self {
            input: input.to_string(),
            e,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.e.variant {
            pest::error::ErrorVariant::ParsingError {
                positives,
                negatives,
            } => {
                if !positives.is_empty() {
                    write!(f, "expected {:?}", positives[0])?;
                    for rule in &positives[1..] {
                        write!(f, ", {:?}", rule)?;
                    }
                    if !negatives.is_empty() {
                        write!(f, "; ")?;
                    }
                }
                if !negatives.is_empty() {
                    write!(f, "unexpected {:?}", negatives[0])?;
                    for rule in &negatives[1..] {
                        write!(f, ", {:?}", rule)?;
                    }
                }
                writeln!(f)?;
                writeln!(f, "{}", self.input)?;
                match &self.e.location {
                    pest::error::InputLocation::Pos(i) => {
                        write!(f, "{}^", " ".repeat(*i))?;
                    }
                    pest::error::InputLocation::Span((i, j)) => {
                        write!(f, "{}{}", " ".repeat(*i), "^".repeat(j - i))?;
                    }
                }
            }
            pest::error::ErrorVariant::CustomError { message } => {
                write!(f, "{}", message)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.e)
    }
}
