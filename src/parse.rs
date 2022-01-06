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
}

impl Direction {
    fn parse(c: u8) -> Option<Self> {
        Some(match c {
            b'>' => Self::Out,
            b'<' => Self::In,
            _ => return None,
        })
    }

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
    fn parse(s: &str) -> Self {
        let (from, to) = s.split_once(&['<', '>'][..]).unwrap();
        let dir = Direction::parse(s.as_bytes()[from.len()]).unwrap();
        let from = if from.is_empty() {
            match dir {
                Direction::In => 0,
                Direction::Out => 1,
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
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::word));
        let word = pair.into_inner().next().unwrap();
        assert!(matches!(
            word.as_rule(),
            Rule::bareword | Rule::single_string | Rule::double_string
        ));
        Self {
            word: word.as_str().to_string(),
            interpolate: matches!(
                word.as_rule(),
                Rule::bareword | Rule::double_string
            ),
            quoted: matches!(
                word.as_rule(),
                Rule::single_string | Rule::double_string
            ),
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
        let exe = Word::build_ast(iter.next().unwrap());
        let (args, redirects): (_, Vec<_>) =
            iter.map(Word::build_ast).partition(|word| {
                word.quoted || !word.word.contains(&['<', '>'][..])
            });
        let redirects =
            redirects.iter().map(|r| Redirect::parse(&r.word)).collect();
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
mod test {
    use super::*;

    macro_rules! c {
        ($input_string:expr, $($pipelines:expr),*) => {
            Commands {
                pipelines: vec![$($pipelines),*],
                input_string: $input_string.to_string(),
            }
        };
    }

    macro_rules! p {
        ($input_string:expr, $($exes:expr),*) => {
            Pipeline {
                exes: vec![$($exes),*],
                input_string: $input_string.to_string(),
            }
        };
    }

    macro_rules! e {
        ($word:expr) => {
            Exe {
                exe: $word,
                args: vec![],
                redirects: vec![], // todo
            }
        };
        ($word:expr, $($args:expr),*) => {
            Exe {
                exe: $word,
                args: vec![$($args),*],
                redirects: vec![], // todo
            }
        };
    }

    macro_rules! w {
        ($word:expr) => {
            Word {
                word: $word.to_string(),
                interpolate: true,
                quoted: false,
            }
        };
        ($word:expr, $interpolate:expr) => {
            Word {
                word: $word.to_string(),
                interpolate: $interpolate,
                quoted: false,
            }
        };
        ($word:expr, $interpolate:expr, $quoted:expr) => {
            Word {
                word: $word.to_string(),
                interpolate: $interpolate,
                quoted: $quoted,
            }
        };
    }

    #[test]
    fn test_basic() {
        assert_eq!(
            &Commands::parse("foo").unwrap(),
            &c!("foo", p!("foo", e!(w!("foo")))),
        );
        assert_eq!(
            &Commands::parse("foo bar").unwrap(),
            &c!("foo bar", p!("foo bar", e!(w!("foo"), w!("bar")))),
        );
        assert_eq!(
            &Commands::parse("foo bar baz").unwrap(),
            &c!(
                "foo bar baz",
                p!("foo bar baz", e!(w!("foo"), w!("bar"), w!("baz")))
            ),
        );
        assert_eq!(
            &Commands::parse("foo | bar").unwrap(),
            &c!("foo | bar", p!("foo | bar", e!(w!("foo")), e!(w!("bar")))),
        );
        assert_eq!(
            &Commands::parse(
                "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar"
            ).unwrap(),
            &c!(
                "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar",
                p!(
                    "command ls",
                    e!(w!("command"), w!("ls"))
                ),
                p!(
                    "perl -E 'say foo' | tr a-z A-Z",
                    e!(w!("perl"), w!("-E"), w!("say foo", false, true)),
                    e!(w!("tr"), w!("a-z"), w!("A-Z"))
                ),
                p!(
                    "builtin echo bar",
                    e!(w!("builtin"), w!("echo"), w!("bar"))
                )
            ),
        );
    }

    #[test]
    fn test_whitespace() {
        assert_eq!(
            &Commands::parse("   foo    ").unwrap(),
            &c!("foo", p!("foo", e!(w!("foo")))),
        );
        assert_eq!(
            &Commands::parse("   foo    # this is a comment").unwrap(),
            &c!("foo", p!("foo", e!(w!("foo")))),
        );
        assert_eq!(
            &Commands::parse("foo    | bar  ").unwrap(),
            &c!(
                "foo    | bar",
                p!("foo    | bar", e!(w!("foo")), e!(w!("bar")))
            ),
        );
        assert_eq!(
            &Commands::parse(
                "  abc def  ghi   |jkl mno|   pqr stu; vwxyz  # comment"
            )
            .unwrap(),
            &c!(
                "abc def  ghi   |jkl mno|   pqr stu; vwxyz",
                p!(
                    "abc def  ghi   |jkl mno|   pqr stu",
                    e!(w!("abc"), w!("def"), w!("ghi")),
                    e!(w!("jkl"), w!("mno")),
                    e!(w!("pqr"), w!("stu"))
                ),
                p!("vwxyz", e!(w!("vwxyz")))
            ),
        );
        assert_eq!(
            &Commands::parse(
                "foo 'bar # baz' \"quux # not a comment\" # comment"
            )
            .unwrap(),
            &c!(
                "foo 'bar # baz' \"quux # not a comment\"",
                p!(
                    "foo 'bar # baz' \"quux # not a comment\"",
                    e!(
                        w!("foo"),
                        w!("bar # baz", false, true),
                        w!("quux # not a comment", true, true)
                    )
                )
            ),
        );
    }
}
