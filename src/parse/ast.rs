use crate::prelude::*;

use pest::Parser as _;

#[derive(pest_derive::Parser)]
#[grammar = "shell.pest"]
struct Shell;

#[derive(Debug, PartialEq, Eq)]
pub struct Commands {
    commands: Vec<Command>,
    input_string: String,
}

impl Commands {
    pub fn parse(full_cmd: &str) -> Result<Self, super::Error> {
        Ok(Self::build_ast(
            Shell::parse(Rule::line, full_cmd)
                .map_err(|e| super::Error::new(full_cmd, anyhow::anyhow!(e)))?
                .next()
                .unwrap()
                .into_inner()
                .next()
                .unwrap(),
        ))
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn input_string(&self) -> &str {
        &self.input_string
    }

    fn build_ast(commands: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(commands.as_rule(), Rule::commands));
        let input_string = commands.as_str().to_string();
        Self {
            commands: commands.into_inner().map(Command::build_ast).collect(),
            input_string,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Pipeline(Pipeline),
    If(Pipeline),
    While(Pipeline),
    For(String, Pipeline),
    End,
}

impl Command {
    fn build_ast(command: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(command.as_rule(), Rule::command));
        let next = command.into_inner().next().unwrap();
        match next.as_rule() {
            Rule::pipeline => Self::Pipeline(Pipeline::build_ast(next)),
            Rule::control => {
                let ty = next.into_inner().next().unwrap();
                match ty.as_rule() {
                    Rule::control_if => Self::If(Pipeline::build_ast(
                        ty.into_inner().next().unwrap(),
                    )),
                    Rule::control_while => Self::While(Pipeline::build_ast(
                        ty.into_inner().next().unwrap(),
                    )),
                    Rule::control_for => {
                        let mut inner = ty.into_inner();
                        let var = inner.next().unwrap();
                        assert!(matches!(var.as_rule(), Rule::bareword));
                        Self::For(
                            var.as_str().to_string(),
                            Pipeline::build_ast(inner.next().unwrap()),
                        )
                    }
                    Rule::control_end => Self::End,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Pipeline {
    exes: Vec<Exe>,
    input_string: String,
}

impl Pipeline {
    pub fn parse(pipeline: &str) -> Result<Self, super::Error> {
        Ok(Self::build_ast(
            Shell::parse(Rule::pipeline, pipeline)
                .map_err(|e| super::Error::new(pipeline, anyhow::anyhow!(e)))?
                .next()
                .unwrap(),
        ))
    }

    pub fn eval(self, env: &Env) -> super::Pipeline {
        super::Pipeline {
            exes: self.exes.into_iter().map(|exe| exe.eval(env)).collect(),
        }
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
struct Exe {
    exe: Word,
    args: Vec<Word>,
    redirects: Vec<Redirect>,
}

impl Exe {
    fn eval(self, env: &Env) -> super::Exe {
        super::Exe {
            exe: std::path::PathBuf::from(self.exe.eval(env)),
            args: self.args.into_iter().map(|arg| arg.eval(env)).collect(),
            redirects: self
                .redirects
                .into_iter()
                .map(|redirect| redirect.eval(env))
                .collect(),
        }
    }

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Word {
    parts: Vec<WordPart>,
}

impl Word {
    fn eval(self, env: &Env) -> String {
        self.parts
            .into_iter()
            .map(|part| part.eval(env))
            .collect::<Vec<_>>()
            .join("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WordPart {
    Var(String),
    Bareword(String),
    DoubleQuoted(String),
    SingleQuoted(String),
}

impl WordPart {
    #[allow(clippy::needless_pass_by_value)]
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        match pair.as_rule() {
            Rule::var => {
                let s = pair.as_str();
                let inner = s.strip_prefix('$').unwrap();
                Self::Var(
                    inner
                        .strip_prefix('{')
                        .map_or(inner, |inner| {
                            inner.strip_suffix('}').unwrap()
                        })
                        .to_string(),
                )
            }
            Rule::bareword => Self::Bareword(strip_escape(pair.as_str())),
            Rule::double_string => {
                Self::DoubleQuoted(strip_escape(pair.as_str()))
            }
            Rule::single_string => {
                Self::SingleQuoted(strip_basic_escape(pair.as_str()))
            }
            _ => unreachable!(),
        }
    }

    fn eval(self, env: &Env) -> String {
        match self {
            Self::Var(name) => env.var(&name),
            Self::Bareword(s)
            | Self::DoubleQuoted(s)
            | Self::SingleQuoted(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Redirect {
    from: std::os::unix::io::RawFd,
    to: Word,
    dir: super::Direction,
}

impl Redirect {
    fn parse(prefix: &str, to: Word) -> Self {
        let (from, dir) = if let Some(from) = prefix.strip_suffix(">>") {
            (from, super::Direction::Append)
        } else if let Some(from) = prefix.strip_suffix('>') {
            (from, super::Direction::Out)
        } else if let Some(from) = prefix.strip_suffix('<') {
            (from, super::Direction::In)
        } else {
            unreachable!()
        };
        let from = if from.is_empty() {
            match dir {
                super::Direction::In => 0,
                super::Direction::Out | super::Direction::Append => 1,
            }
        } else {
            from.parse().unwrap()
        };
        Self { from, to, dir }
    }

    fn eval(self, env: &Env) -> super::Redirect {
        let to = if self.to.parts.len() == 1 {
            if let WordPart::Bareword(s) = &self.to.parts[0] {
                if let Some(fd) = s.strip_prefix('&') {
                    super::RedirectTarget::Fd(fd.parse().unwrap())
                } else {
                    super::RedirectTarget::File(std::path::PathBuf::from(
                        self.to.eval(env),
                    ))
                }
            } else {
                super::RedirectTarget::File(std::path::PathBuf::from(
                    self.to.eval(env),
                ))
            }
        } else {
            super::RedirectTarget::File(std::path::PathBuf::from(
                self.to.eval(env),
            ))
        };
        super::Redirect {
            from: self.from,
            to,
            dir: self.dir,
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
        let mut inner = pair.into_inner().peekable();
        let prefix = if matches!(
            inner.peek().map(pest::iterators::Pair::as_rule),
            Some(Rule::redir_prefix)
        ) {
            Some(inner.next().unwrap().as_str().trim().to_string())
        } else {
            None
        };
        let word = Word {
            parts: inner.map(WordPart::build_ast).collect(),
        };
        if let Some(prefix) = prefix {
            Self::Redirect(Redirect::parse(&prefix, word))
        } else {
            Self::Word(word)
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

#[cfg(test)]
#[path = "test_ast.rs"]
mod test;
