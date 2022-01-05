use pest::Parser as _;

#[derive(pest_derive::Parser)]
#[grammar = "shell.pest"]
struct Shell;

#[derive(Debug, Clone)]
pub enum RedirectTarget {
    Fd(std::os::unix::io::RawFd),
    File(std::path::PathBuf),
}

#[derive(Debug, Clone)]
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
}

#[derive(Debug, Clone)]
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

#[derive(Debug)]
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

#[derive(Debug)]
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

#[derive(Debug)]
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

#[derive(Debug)]
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
