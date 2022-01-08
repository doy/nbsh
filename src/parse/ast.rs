use crate::prelude::*;

use pest::Parser as _;

#[derive(pest_derive::Parser)]
#[grammar = "shell.pest"]
struct Shell;

#[derive(Debug, PartialEq, Eq)]
pub struct Commands {
    pipelines: Vec<Pipeline>,
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

    pub fn eval(self, env: &Env) -> super::Commands {
        super::Commands {
            pipelines: self
                .pipelines
                .into_iter()
                .map(|pipeline| pipeline.eval(env))
                .collect(),
        }
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
            input_string: self.input_string,
        }
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
    word: String,
    interpolate: bool,
    quoted: bool,
}

impl Word {
    fn parse(s: &str, rule: Rule) -> Self {
        let interpolate =
            matches!(rule, Rule::bareword | Rule::double_string);
        let quoted =
            matches!(rule, Rule::single_string | Rule::double_string);
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

    fn eval(self, _env: &Env) -> String {
        // TODO
        self.word
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Redirect {
    from: std::os::unix::io::RawFd,
    to: RedirectTarget,
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
        #[allow(clippy::option_if_let_else)]
        let to = if let Some(fd) = to.word.strip_prefix('&') {
            RedirectTarget::Fd(fd.parse().unwrap())
        } else {
            RedirectTarget::File(to)
        };
        Self { from, to, dir }
    }

    fn eval(self, env: &Env) -> super::Redirect {
        super::Redirect {
            from: self.from,
            to: self.to.eval(env),
            dir: self.dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RedirectTarget {
    Fd(std::os::unix::io::RawFd),
    File(Word),
}

impl RedirectTarget {
    fn eval(self, env: &Env) -> super::RedirectTarget {
        match self {
            Self::Fd(fd) => super::RedirectTarget::Fd(fd),
            Self::File(path) => super::RedirectTarget::File(
                std::path::PathBuf::from(path.eval(env)),
            ),
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
        let word = Word::parse(word.as_str(), word.as_rule());
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
impl From<std::os::unix::io::RawFd> for RedirectTarget {
    fn from(fd: std::os::unix::io::RawFd) -> Self {
        Self::Fd(fd)
    }
}

#[allow(clippy::fallible_impl_from)]
#[cfg(test)]
impl From<Word> for RedirectTarget {
    fn from(word: Word) -> Self {
        Self::File(word)
    }
}

#[cfg(test)]
macro_rules! c {
        ($input_string:expr, $($pipelines:expr),*) => {
            Commands {
                pipelines: vec![$($pipelines),*],
                input_string: $input_string.to_string(),
            }
        };
    }

#[cfg(test)]
macro_rules! p {
        ($input_string:expr, $($exes:expr),*) => {
            Pipeline {
                exes: vec![$($exes),*],
                input_string: $input_string.to_string(),
            }
        };
    }

#[cfg(test)]
macro_rules! e {
        ($word:expr) => {
            Exe {
                exe: $word,
                args: vec![],
                redirects: vec![],
            }
        };
        ($word:expr, $($args:expr),*) => {
            Exe {
                exe: $word,
                args: vec![$($args),*],
                redirects: vec![],
            }
        };
        ($word:expr ; $($redirects:expr),*) => {
            Exe {
                exe: $word,
                args: vec![],
                redirects: vec![$($redirects),*],
            }
        };
        ($word:expr, $($args:expr),* ; $($redirects:expr),*) => {
            Exe {
                exe: $word,
                args: vec![$($args),*],
                redirects: vec![$($redirects),*],
            }
        };
    }

#[cfg(test)]
macro_rules! r {
    ($from:literal, $to:expr, $dir:ident) => {
        Redirect {
            from: $from,
            to: $to.into(),
            dir: super::Direction::$dir,
        }
    };
}

#[cfg(test)]
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

#[cfg(test)]
macro_rules! parse_eq {
    ($line:literal, $parsed:expr) => {
        assert_eq!(&Commands::parse($line).unwrap(), &$parsed)
    };
}

#[test]
fn test_basic() {
    parse_eq!("foo", c!("foo", p!("foo", e!(w!("foo")))));
    parse_eq!(
        "foo bar",
        c!("foo bar", p!("foo bar", e!(w!("foo"), w!("bar"))))
    );
    parse_eq!(
        "foo bar baz",
        c!(
            "foo bar baz",
            p!("foo bar baz", e!(w!("foo"), w!("bar"), w!("baz")))
        )
    );
    parse_eq!(
        "foo | bar",
        c!("foo | bar", p!("foo | bar", e!(w!("foo")), e!(w!("bar"))))
    );
    parse_eq!(
        "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar",
        c!(
            "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar",
            p!("command ls", e!(w!("command"), w!("ls"))),
            p!(
                "perl -E 'say foo' | tr a-z A-Z",
                e!(w!("perl"), w!("-E"), w!("say foo", false, true)),
                e!(w!("tr"), w!("a-z"), w!("A-Z"))
            ),
            p!("builtin echo bar", e!(w!("builtin"), w!("echo"), w!("bar")))
        )
    );
}

#[test]
fn test_whitespace() {
    parse_eq!("   foo    ", c!("foo", p!("foo", e!(w!("foo")))));
    parse_eq!(
        "   foo    # this is a comment",
        c!("foo", p!("foo", e!(w!("foo"))))
    );
    parse_eq!("foo#comment", c!("foo", p!("foo", e!(w!("foo")))));
    parse_eq!(
        "foo;bar|baz;quux#comment",
        c!(
            "foo;bar|baz;quux",
            p!("foo", e!(w!("foo"))),
            p!("bar|baz", e!(w!("bar")), e!(w!("baz"))),
            p!("quux", e!(w!("quux")))
        )
    );
    parse_eq!(
        "foo    | bar  ",
        c!(
            "foo    | bar",
            p!("foo    | bar", e!(w!("foo")), e!(w!("bar")))
        )
    );
    parse_eq!(
        "  abc def  ghi   |jkl mno|   pqr stu; vwxyz  # comment",
        c!(
            "abc def  ghi   |jkl mno|   pqr stu; vwxyz",
            p!(
                "abc def  ghi   |jkl mno|   pqr stu",
                e!(w!("abc"), w!("def"), w!("ghi")),
                e!(w!("jkl"), w!("mno")),
                e!(w!("pqr"), w!("stu"))
            ),
            p!("vwxyz", e!(w!("vwxyz")))
        )
    );
    parse_eq!(
        "foo 'bar # baz' \"quux # not a comment\" # comment",
        c!(
            "foo 'bar # baz' \"quux # not a comment\"",
            p!(
                "foo 'bar # baz' \"quux # not a comment\"",
                e!(
                    w!("foo"),
                    w!("bar # baz", false, true),
                    w!("quux # not a comment", true, true)
                )
            )
        )
    );
}

#[test]
fn test_redirect() {
    parse_eq!(
        "foo > bar",
        c!(
            "foo > bar",
            p!("foo > bar", e!(w!("foo") ; r!(1, w!("bar"), Out)))
        )
    );
    parse_eq!(
        "foo <bar",
        c!(
            "foo <bar",
            p!("foo <bar", e!(w!("foo") ; r!(0, w!("bar"), In)))
        )
    );
    parse_eq!(
        "foo > /dev/null 2>&1",
        c!(
            "foo > /dev/null 2>&1",
            p!(
                "foo > /dev/null 2>&1",
                e!(w!("foo") ; r!(1, w!("/dev/null"), Out), r!(2, 1, Out))
            )
        )
    );
    parse_eq!(
        "foo >>bar",
        c!(
            "foo >>bar",
            p!("foo >>bar", e!(w!("foo") ; r!(1, w!("bar"), Append)))
        )
    );
    parse_eq!(
        "foo >> bar",
        c!(
            "foo >> bar",
            p!("foo >> bar", e!(w!("foo") ; r!(1, w!("bar"), Append)))
        )
    );
    parse_eq!(
        "foo > 'bar baz'",
        c!(
            "foo > 'bar baz'",
            p!(
                "foo > 'bar baz'",
                e!(w!("foo") ; r!(1, w!("bar baz", false, true), Out))
            )
        )
    );
}

#[test]
fn test_escape() {
    parse_eq!(
        "foo\\ bar",
        c!("foo\\ bar", p!("foo\\ bar", e!(w!("foo bar"))))
    );
    parse_eq!(
        "'foo\\ bar'",
        c!(
            "'foo\\ bar'",
            p!("'foo\\ bar'", e!(w!("foo\\ bar", false, true)))
        )
    );
    parse_eq!(
        "\"foo\\ bar\"",
        c!(
            "\"foo\\ bar\"",
            p!("\"foo\\ bar\"", e!(w!("foo bar", true, true)))
        )
    );
    parse_eq!(
        "\"foo\\\"bar\"",
        c!(
            "\"foo\\\"bar\"",
            p!("\"foo\\\"bar\"", e!(w!("foo\"bar", true, true)))
        )
    );
    parse_eq!(
        "'foo\\'bar\\\\'",
        c!(
            "'foo\\'bar\\\\'",
            p!("'foo\\'bar\\\\'", e!(w!("foo'bar\\", false, true)))
        )
    );
    parse_eq!(
        "foo > bar\\ baz",
        c!(
            "foo > bar\\ baz",
            p!("foo > bar\\ baz", e!(w!("foo") ; r!(1, w!("bar baz"), Out)))
        )
    );
}
