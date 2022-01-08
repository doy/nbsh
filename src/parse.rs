use pest::Parser as _;

#[derive(pest_derive::Parser)]
#[grammar = "shell.pest"]
struct Shell;

#[derive(Debug, Clone, PartialEq, Eq)]
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
            crate::parse::Direction::In => nix::fcntl::open(
                path,
                OFlag::O_NOCTTY | OFlag::O_RDONLY,
                Mode::empty(),
            )?,
            crate::parse::Direction::Out => nix::fcntl::open(
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
            crate::parse::Direction::Append => nix::fcntl::open(
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub from: std::os::unix::io::RawFd,
    pub to: RedirectTarget,
    pub dir: Direction,
}

impl Redirect {
    fn parse(prefix: &str, to: &str) -> Self {
        let (from, dir) = if let Some(from) = prefix.strip_suffix(">>") {
            (from, Direction::Append)
        } else if let Some(from) = prefix.strip_suffix('>') {
            (from, Direction::Out)
        } else if let Some(from) = prefix.strip_suffix('<') {
            (from, Direction::In)
        } else {
            unreachable!()
        };
        let from = if from.is_empty() {
            match dir {
                Direction::In => 0,
                Direction::Out | Direction::Append => 1,
            }
        } else {
            from.parse().unwrap()
        };
        let to = to.strip_prefix('&').map_or_else(
            || RedirectTarget::File(to.into()),
            |fd| RedirectTarget::Fd(fd.parse().unwrap()),
        );
        Self { from, to, dir }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Word {
    word: String,
    interpolate: bool,
    quoted: bool,
}

impl Word {
    fn parse(s: &str, interpolate: bool, quoted: bool) -> Self {
        let mut word_str = s.to_string();
        if interpolate {
            word_str = strip_escape(&word_str);
        } else {
            word_str = strip_basic_escape(&word_str);
        }
        Self {
            word: word_str,
            interpolate,
            quoted,
        }
    }
}

enum WordOrRedirect {
    Word(Word),
    Redirect(Redirect),
}

impl WordOrRedirect {
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::word));
        let mut inner = pair.into_inner();
        let mut word = inner.next().unwrap();
        let mut prefix = None;
        if matches!(word.as_rule(), Rule::redir_prefix) {
            prefix = Some(word.as_str().trim().to_string());
            word = inner.next().unwrap();
        }
        assert!(matches!(
            word.as_rule(),
            Rule::bareword | Rule::single_string | Rule::double_string
        ));
        let word = Word::parse(
            word.as_str(),
            matches!(word.as_rule(), Rule::bareword | Rule::double_string),
            matches!(
                word.as_rule(),
                Rule::single_string | Rule::double_string
            ),
        );
        if let Some(prefix) = prefix {
            Self::Redirect(Redirect::parse(&prefix, &word.word))
        } else {
            Self::Word(word)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Exe {
    exe: Word,
    args: Vec<Word>,
    redirects: Vec<Redirect>,
}

impl Exe {
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::exe));
        let mut iter = pair.into_inner();
        let exe = match WordOrRedirect::build_ast(iter.next().unwrap()) {
            WordOrRedirect::Word(word) => word,
            WordOrRedirect::Redirect(_) => todo!(),
        };
        let (args, redirects): (_, Vec<_>) = iter
            .map(WordOrRedirect::build_ast)
            .partition(|word| matches!(word, WordOrRedirect::Word(_)));
        let args = args
            .into_iter()
            .map(|word| match word {
                WordOrRedirect::Word(word) => word,
                WordOrRedirect::Redirect(_) => unreachable!(),
            })
            .collect();
        let redirects = redirects
            .into_iter()
            .map(|word| match word {
                WordOrRedirect::Word(_) => unreachable!(),
                WordOrRedirect::Redirect(redirect) => redirect,
            })
            .collect();
        Self {
            exe,
            args,
            redirects,
        }
    }

    pub fn exe(&self) -> &str {
        &self.exe.word
    }

    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(|arg| arg.word.as_ref())
    }

    pub fn redirects(&self) -> &[Redirect] {
        &self.redirects
    }

    pub fn shift(&mut self) {
        self.exe = self.args.remove(0);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Pipeline {
    exes: Vec<Exe>,
    input_string: String,
}

impl Pipeline {
    pub fn parse(pipeline: &str) -> Result<Self, Error> {
        Ok(Self::build_ast(
            Shell::parse(Rule::pipeline, pipeline)
                .map_err(|e| Error::new(pipeline, anyhow::anyhow!(e)))?
                .next()
                .unwrap(),
        ))
    }

    pub fn into_exes(self) -> impl Iterator<Item = Exe> {
        self.exes.into_iter()
    }

    pub fn input_string(&self) -> &str {
        &self.input_string
    }

    fn build_ast(pipeline: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pipeline.as_rule(), Rule::pipeline));
        let input_string = pipeline.as_str().to_string();
        Self {
            exes: pipeline.into_inner().map(Exe::build_ast).collect(),
            input_string,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Commands {
    pipelines: Vec<Pipeline>,
    input_string: String,
}

impl Commands {
    pub fn parse(full_cmd: &str) -> Result<Self, Error> {
        Ok(Self::build_ast(
            Shell::parse(Rule::line, full_cmd)
                .map_err(|e| Error::new(full_cmd, anyhow::anyhow!(e)))?
                .next()
                .unwrap()
                .into_inner()
                .next()
                .unwrap(),
        ))
    }

    pub fn pipelines(&self) -> &[Pipeline] {
        &self.pipelines
    }

    pub fn input_string(&self) -> &str {
        &self.input_string
    }

    fn build_ast(commands: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(commands.as_rule(), Rule::commands));
        let input_string = commands.as_str().to_string();
        Self {
            pipelines: commands
                .into_inner()
                .map(Pipeline::build_ast)
                .collect(),
            input_string,
        }
    }
}

fn strip_escape(s: &str) -> String {
    let mut new = String::new();
    let mut escape = false;
    for c in s.chars() {
        if escape {
            new.push(c);
            escape = false;
        } else {
            match c {
                '\\' => escape = true,
                _ => new.push(c),
            }
        }
    }
    new
}

fn strip_basic_escape(s: &str) -> String {
    let mut new = String::new();
    let mut escape = false;
    for c in s.chars() {
        if escape {
            match c {
                '\\' | '\'' => {}
                _ => new.push('\\'),
            }
            new.push(c);
            escape = false;
        } else {
            match c {
                '\\' => escape = true,
                _ => new.push(c),
            }
        }
    }
    new
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

    pub fn into_input(self) -> String {
        self.input
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

#[cfg(test)]
#[path = "test_parse.rs"]
mod test;
