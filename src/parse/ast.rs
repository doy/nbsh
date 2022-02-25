use crate::prelude::*;

use pest::Parser as _;

#[derive(pest_derive::Parser)]
#[grammar = "shell.pest"]
struct Shell;

#[derive(Debug, PartialEq, Eq)]
pub struct Commands {
    commands: Vec<Command>,
}

impl Commands {
    pub fn parse(full_cmd: &str) -> Result<Self, super::Error> {
        Ok(Self::build_ast(
            Shell::parse(Rule::line, full_cmd)
                .map_err(|e| super::Error::new(full_cmd, e))?
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

    fn build_ast(commands: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(commands.as_rule(), Rule::commands));
        Self {
            commands: commands.into_inner().map(Command::build_ast).collect(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Pipeline(Pipeline),
    If(Pipeline),
    While(Pipeline),
    For(String, Vec<Word>),
    Else(Option<Pipeline>),
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
                        let list = inner.next().unwrap();
                        assert!(matches!(list.as_rule(), Rule::list));
                        let vals =
                            list.into_inner().map(Word::build_ast).collect();
                        Self::For(var.as_str().to_string(), vals)
                    }
                    Rule::control_else => Self::Else(
                        ty.into_inner().next().map(Pipeline::build_ast),
                    ),
                    Rule::control_end => Self::End,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    exes: Vec<Exe>,
    span: (usize, usize),
}

impl Pipeline {
    pub async fn eval(self, env: &Env) -> anyhow::Result<super::Pipeline> {
        Ok(super::Pipeline {
            exes: self
                .exes
                .into_iter()
                .map(|exe| exe.eval(env))
                .collect::<futures_util::stream::FuturesOrdered<_>>()
                .try_collect()
                .await?,
        })
    }

    pub fn span(&self) -> (usize, usize) {
        self.span
    }

    fn build_ast(pipeline: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pipeline.as_rule(), Rule::pipeline));
        let span = (pipeline.as_span().start(), pipeline.as_span().end());
        Self {
            exes: pipeline.into_inner().map(Exe::build_ast).collect(),
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Exe {
    exe: Word,
    args: Vec<Word>,
    redirects: Vec<Redirect>,
}

impl Exe {
    async fn eval(self, env: &Env) -> anyhow::Result<super::Exe> {
        let exe = self.exe.eval(env).await?;
        assert_eq!(exe.len(), 1); // TODO
        let exe = &exe[0];
        Ok(super::Exe {
            exe: std::path::PathBuf::from(exe),
            args: self
                .args
                .into_iter()
                .map(|arg| async {
                    arg.eval(env).await.map(IntoIterator::into_iter)
                })
                .collect::<futures_util::stream::FuturesOrdered<_>>()
                .try_collect::<Vec<_>>()
                .await?
                .into_iter()
                .flatten()
                .collect(),
            redirects: self
                .redirects
                .into_iter()
                .map(|arg| arg.eval(env))
                .collect::<futures_util::stream::FuturesOrdered<_>>()
                .try_collect()
                .await?,
        })
    }

    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::subshell | Rule::exe));
        if matches!(pair.as_rule(), Rule::subshell) {
            let mut iter = pair.into_inner();
            let commands = iter.next().unwrap();
            assert!(matches!(commands.as_rule(), Rule::commands));
            let redirects = iter.map(Redirect::build_ast).collect();
            return Self {
                exe: Word {
                    parts: vec![WordPart::SingleQuoted(
                        std::env::current_exe()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string(),
                    )],
                },
                args: vec![
                    Word {
                        parts: vec![WordPart::SingleQuoted("-c".to_string())],
                    },
                    Word {
                        parts: vec![WordPart::SingleQuoted(
                            commands.as_str().to_string(),
                        )],
                    },
                ],
                redirects,
            };
        }
        let mut iter = pair.into_inner();
        let exe = iter.next().unwrap();
        let exe = match exe.as_rule() {
            Rule::word => Word::build_ast(exe),
            Rule::redirect => todo!(),
            _ => unreachable!(),
        };
        let mut args = vec![];
        let mut redirects = vec![];
        for arg in iter {
            match arg.as_rule() {
                Rule::word => args.push(Word::build_ast(arg)),
                Rule::redirect => redirects.push(Redirect::build_ast(arg)),
                _ => unreachable!(),
            }
        }
        Self {
            exe,
            args,
            redirects,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    parts: Vec<WordPart>,
}

impl Word {
    pub async fn eval(self, env: &Env) -> anyhow::Result<Vec<String>> {
        let mut opts = glob::MatchOptions::new();
        opts.require_literal_separator = true;
        opts.require_literal_leading_dot = true;

        let mut alternations: Vec<Vec<Vec<WordPart>>> = vec![];
        let mut cur: Vec<WordPart> = vec![];
        for part in self.parts {
            if let WordPart::Alternation(words) = part {
                if !cur.is_empty() {
                    alternations.push(vec![cur.clone()]);
                    cur.clear();
                }
                alternations
                    .push(words.into_iter().map(|word| word.parts).collect());
            } else {
                cur.push(part.clone());
            }
        }
        if !cur.is_empty() {
            alternations.push(vec![cur]);
        }
        let mut words: Vec<Vec<WordPart>> = std::iter::repeat(vec![])
            .take(alternations.iter().map(Vec::len).product())
            .collect();
        for i in 0..words.len() {
            let mut len = words.len();
            for alternation in &alternations {
                let idx = (i * alternation.len() / len) % alternation.len();
                words[i].extend(alternation[idx].clone().into_iter());
                len /= alternation.len();
            }
        }

        let mut expanded_words = vec![];
        for word in words {
            let mut s = String::new();
            let mut pat = String::new();
            let mut is_glob = false;
            let initial_bareword = word
                .get(0)
                .map_or(false, |part| matches!(part, WordPart::Bareword(_)));
            for part in word {
                match part {
                    WordPart::Alternation(_) => unreachable!(),
                    WordPart::Bareword(_) => {
                        let part = part.eval(env).await;
                        s.push_str(&part);
                        pat.push_str(&part);
                        if part.contains(&['*', '?', '['][..]) {
                            is_glob = true;
                        }
                    }
                    WordPart::Substitution(_)
                    | WordPart::Var(_)
                    | WordPart::DoubleQuoted(_)
                    | WordPart::SingleQuoted(_) => {
                        let part = part.eval(env).await;
                        s.push_str(&part);
                        pat.push_str(&glob::Pattern::escape(&part));
                    }
                }
            }
            if initial_bareword {
                s = expand_home(&s)?;
                pat = expand_home(&pat)?;
            }
            if is_glob {
                let mut found = false;
                for file in glob::glob_with(&pat, opts)? {
                    let file = file?;
                    let s = file.to_str().unwrap();
                    if s == "."
                        || s == ".."
                        || s.ends_with("/.")
                        || s.ends_with("/..")
                    {
                        continue;
                    }
                    found = true;
                    expanded_words.push(s.to_string());
                }
                if !found {
                    anyhow::bail!("no matches for {}", s);
                }
            } else {
                expanded_words.push(s);
            }
        }
        Ok(expanded_words)
    }

    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(
            pair.as_rule(),
            Rule::word | Rule::alternation_word
        ));
        Self {
            parts: pair.into_inner().flat_map(WordPart::build_ast).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WordPart {
    Alternation(Vec<Word>),
    Substitution(String),
    Var(String),
    Bareword(String),
    DoubleQuoted(String),
    SingleQuoted(String),
}

impl WordPart {
    async fn eval(self, env: &Env) -> String {
        match self {
            Self::Alternation(_) => unreachable!(),
            Self::Substitution(commands) => {
                let mut cmd = tokio::process::Command::new(
                    std::env::current_exe().unwrap(),
                );
                cmd.args(&["-c", &commands]);
                cmd.stdin(std::process::Stdio::inherit());
                cmd.stderr(std::process::Stdio::inherit());
                let mut out =
                    String::from_utf8(cmd.output().await.unwrap().stdout)
                        .unwrap();
                if out.ends_with('\n') {
                    out.truncate(out.len() - 1);
                }
                out
            }
            Self::Var(name) => {
                env.var(&name).unwrap_or_else(|| "".to_string())
            }
            Self::Bareword(s)
            | Self::DoubleQuoted(s)
            | Self::SingleQuoted(s) => s,
        }
    }

    fn build_ast(
        pair: pest::iterators::Pair<Rule>,
    ) -> impl Iterator<Item = Self> + '_ {
        assert!(matches!(
            pair.as_rule(),
            Rule::word_part | Rule::alternation_word_part
        ));
        pair.into_inner().map(|pair| match pair.as_rule() {
            Rule::substitution => {
                let commands = pair.into_inner().next().unwrap();
                assert!(matches!(commands.as_rule(), Rule::commands));
                Self::Substitution(commands.as_str().to_string())
            }
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
            Rule::bareword | Rule::alternation_bareword => {
                Self::Bareword(strip_escape(pair.as_str()))
            }
            Rule::double_string => {
                Self::DoubleQuoted(strip_escape(pair.as_str()))
            }
            Rule::single_string => {
                Self::SingleQuoted(strip_basic_escape(pair.as_str()))
            }
            Rule::alternation => Self::Alternation(
                pair.into_inner().map(Word::build_ast).collect(),
            ),
            _ => unreachable!(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Redirect {
    from: std::os::unix::io::RawFd,
    to: Word,
    dir: super::Direction,
}

impl Redirect {
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::redirect));
        let mut iter = pair.into_inner();

        let prefix = iter.next().unwrap().as_str();
        let (from, dir) = prefix.strip_suffix(">>").map_or_else(
            || {
                prefix.strip_suffix('>').map_or_else(
                    || {
                        (
                            prefix.strip_suffix('<').unwrap(),
                            super::Direction::In,
                        )
                    },
                    |from| (from, super::Direction::Out),
                )
            },
            |from| (from, super::Direction::Append),
        );
        let from = if from.is_empty() {
            match dir {
                super::Direction::In => 0,
                super::Direction::Out | super::Direction::Append => 1,
            }
        } else {
            parse_fd(from)
        };

        let to = Word::build_ast(iter.next().unwrap());

        Self { from, to, dir }
    }

    async fn eval(self, env: &Env) -> anyhow::Result<super::Redirect> {
        let to = if self.to.parts.len() == 1 {
            if let WordPart::Bareword(s) = &self.to.parts[0] {
                if let Some(fd) = s.strip_prefix('&') {
                    super::RedirectTarget::Fd(parse_fd(fd))
                } else {
                    let to = self.to.eval(env).await?;
                    assert_eq!(to.len(), 1); // TODO
                    let to = &to[0];
                    super::RedirectTarget::File(std::path::PathBuf::from(to))
                }
            } else {
                let to = self.to.eval(env).await?;
                assert_eq!(to.len(), 1); // TODO
                let to = &to[0];
                super::RedirectTarget::File(std::path::PathBuf::from(to))
            }
        } else {
            let to = self.to.eval(env).await?;
            assert_eq!(to.len(), 1); // TODO
            let to = &to[0];
            super::RedirectTarget::File(std::path::PathBuf::from(to))
        };
        Ok(super::Redirect {
            from: self.from,
            to,
            dir: self.dir,
        })
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

fn parse_fd(s: &str) -> std::os::unix::io::RawFd {
    match s {
        "in" => 0,
        "out" => 1,
        "err" => 2,
        _ => s.parse().unwrap(),
    }
}

fn expand_home(dir: &str) -> anyhow::Result<String> {
    if dir.starts_with('~') {
        let path: std::path::PathBuf = dir.into();
        if let std::path::Component::Normal(prefix) =
            path.components().next().unwrap()
        {
            let prefix_bytes = prefix.as_bytes();
            let name = if prefix_bytes == b"~" {
                None
            } else {
                Some(std::ffi::OsStr::from_bytes(&prefix_bytes[1..]))
            };
            if let Some(home) = home(name) {
                Ok(home
                    .join(path.strip_prefix(prefix).unwrap())
                    .to_str()
                    .unwrap()
                    .to_string())
            } else {
                anyhow::bail!(
                    "no such user: {}",
                    name.map(std::ffi::OsStr::to_string_lossy)
                        .as_ref()
                        .unwrap_or(&std::borrow::Cow::Borrowed("(deleted)"))
                );
            }
        } else {
            unreachable!()
        }
    } else {
        Ok(dir.to_string())
    }
}

fn home(user: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    let user = user.map_or_else(
        || users::get_user_by_uid(users::get_current_uid()),
        users::get_user_by_name,
    );
    user.map(|user| user.home_dir().to_path_buf())
}

#[cfg(test)]
#[path = "test_ast.rs"]
mod test;
