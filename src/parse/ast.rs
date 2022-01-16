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
    pub fn eval(self, env: &Env) -> anyhow::Result<super::Pipeline> {
        Ok(super::Pipeline {
            exes: self
                .exes
                .into_iter()
                .map(|exe| exe.eval(env))
                .collect::<Result<_, _>>()?,
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
    fn eval(self, env: &Env) -> anyhow::Result<super::Exe> {
        let exe = self.exe.eval(env)?;
        assert_eq!(exe.len(), 1); // TODO
        let exe = &exe[0];
        Ok(super::Exe {
            exe: std::path::PathBuf::from(exe),
            args: self
                .args
                .into_iter()
                .map(|arg| arg.eval(env).map(IntoIterator::into_iter))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect(),
            redirects: self
                .redirects
                .into_iter()
                .map(|arg| arg.eval(env))
                .collect::<Result<_, _>>()?,
        })
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
pub struct Word {
    parts: Vec<WordPart>,
}

impl Word {
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(
            pair.as_rule(),
            Rule::word | Rule::alternation_word
        ));
        Self {
            parts: pair.into_inner().flat_map(WordPart::build_ast).collect(),
        }
    }

    pub fn eval(self, env: &Env) -> anyhow::Result<Vec<String>> {
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
                        let part = part.eval(env);
                        s.push_str(&part);
                        pat.push_str(&part);
                        if part.contains(&['*', '?', '['][..]) {
                            is_glob = true;
                        }
                    }
                    WordPart::Var(_)
                    | WordPart::DoubleQuoted(_)
                    | WordPart::SingleQuoted(_) => {
                        let part = part.eval(env);
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WordPart {
    Alternation(Vec<Word>),
    Var(String),
    Bareword(String),
    DoubleQuoted(String),
    SingleQuoted(String),
}

impl WordPart {
    #[allow(clippy::needless_pass_by_value)]
    fn build_ast(
        pair: pest::iterators::Pair<Rule>,
    ) -> impl Iterator<Item = Self> + '_ {
        assert!(matches!(
            pair.as_rule(),
            Rule::word_part | Rule::alternation_word_part
        ));
        pair.into_inner().map(|pair| match pair.as_rule() {
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

    fn eval(self, env: &Env) -> String {
        match self {
            Self::Alternation(_) => unreachable!(),
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
            parse_fd(from)
        };
        Self { from, to, dir }
    }

    fn eval(self, env: &Env) -> anyhow::Result<super::Redirect> {
        let to = if self.to.parts.len() == 1 {
            if let WordPart::Bareword(s) = &self.to.parts[0] {
                if let Some(fd) = s.strip_prefix('&') {
                    super::RedirectTarget::Fd(parse_fd(fd))
                } else {
                    let to = self.to.eval(env)?;
                    assert_eq!(to.len(), 1); // TODO
                    let to = &to[0];
                    super::RedirectTarget::File(std::path::PathBuf::from(to))
                }
            } else {
                let to = self.to.eval(env)?;
                assert_eq!(to.len(), 1); // TODO
                let to = &to[0];
                super::RedirectTarget::File(std::path::PathBuf::from(to))
            }
        } else {
            let to = self.to.eval(env)?;
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

enum WordOrRedirect {
    Word(Word),
    Redirect(Redirect),
}

impl WordOrRedirect {
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::word_or_redirect));
        let mut inner = pair.into_inner().peekable();
        let prefix = if matches!(
            inner.peek().map(pest::iterators::Pair::as_rule),
            Some(Rule::redir_prefix)
        ) {
            Some(inner.next().unwrap().as_str().trim().to_string())
        } else {
            None
        };
        let word = Word::build_ast(inner.next().unwrap());
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
