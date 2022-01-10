use super::*;

impl From<Pipeline> for Command {
    fn from(pipeline: Pipeline) -> Self {
        Self::Pipeline(pipeline)
    }
}

macro_rules! cs {
    ($($commands:expr),*) => {
        Commands {
            commands: [$($commands),*]
                .into_iter()
                .map(|c| c.into())
                .collect(),
        }
    };
}

macro_rules! p {
    ($($exes:expr),*) => {
        Pipeline {
            exes: vec![$($exes),*],
        }
    };
}

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

macro_rules! r {
    ($from:literal, $to:expr, $dir:ident) => {
        Redirect {
            from: $from,
            to: $to,
            dir: super::super::Direction::$dir,
        }
    };
}

macro_rules! w {
    ($word:literal) => {
        Word {
            parts: vec![WordPart::Bareword($word.to_string())],
        }
    };
    ($($word:expr),*) => {
        Word {
            parts: vec![$($word),*],
        }
    }
}

macro_rules! wpv {
    ($var:literal) => {
        WordPart::Var($var.to_string())
    };
}

macro_rules! wpb {
    ($bareword:literal) => {
        WordPart::Bareword($bareword.to_string())
    };
}

macro_rules! wpd {
    ($doublequoted:literal) => {
        WordPart::DoubleQuoted($doublequoted.to_string())
    };
}

macro_rules! wps {
    ($singlequoted:literal) => {
        WordPart::SingleQuoted($singlequoted.to_string())
    };
}

macro_rules! parse_eq {
    ($line:literal, $parsed:expr) => {
        assert_eq!(&Commands::parse($line).unwrap(), &$parsed)
    };
}

#[test]
fn test_basic() {
    parse_eq!("foo", cs!(p!(e!(w!("foo")))));
    parse_eq!("foo bar", cs!(p!(e!(w!("foo"), w!("bar")))));
    parse_eq!("foo bar baz", cs!(p!(e!(w!("foo"), w!("bar"), w!("baz")))));
    parse_eq!("foo | bar", cs!(p!(e!(w!("foo")), e!(w!("bar")))));
    parse_eq!(
        "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar",
        cs!(
            p!(e!(w!("command"), w!("ls"))),
            p!(
                e!(w!("perl"), w!("-E"), w!(wps!("say foo"))),
                e!(w!("tr"), w!("a-z"), w!("A-Z"))
            ),
            p!(e!(w!("builtin"), w!("echo"), w!("bar")))
        )
    );
}

#[test]
fn test_whitespace() {
    parse_eq!("   foo    ", cs!(p!(e!(w!("foo")))));
    parse_eq!("   foo    # this is a comment", cs!(p!(e!(w!("foo")))));
    parse_eq!("foo#comment", cs!(p!(e!(w!("foo")))));
    parse_eq!(
        "foo;bar|baz;quux#comment",
        cs!(
            p!(e!(w!("foo"))),
            p!(e!(w!("bar")), e!(w!("baz"))),
            p!(e!(w!("quux")))
        )
    );
    parse_eq!("foo    | bar  ", cs!(p!(e!(w!("foo")), e!(w!("bar")))));
    parse_eq!(
        "  abc def  ghi   |jkl mno|   pqr stu; vwxyz  # comment",
        cs!(
            p!(
                e!(w!("abc"), w!("def"), w!("ghi")),
                e!(w!("jkl"), w!("mno")),
                e!(w!("pqr"), w!("stu"))
            ),
            p!(e!(w!("vwxyz")))
        )
    );
    parse_eq!(
        "foo 'bar # baz' \"quux # not a comment\" # comment",
        cs!(p!(e!(
            w!("foo"),
            w!(wps!("bar # baz")),
            w!(wpd!("quux # not a comment"))
        )))
    );
}

#[test]
fn test_redirect() {
    parse_eq!("foo > bar", cs!(p!(e!(w!("foo") ; r!(1, w!("bar"), Out)))));
    parse_eq!("foo <bar", cs!(p!(e!(w!("foo") ; r!(0, w!("bar"), In)))));
    parse_eq!(
        "foo > /dev/null 2>&1",
        cs!(p!(e!(
            w!("foo") ;
            r!(1, w!("/dev/null"), Out), r!(2, w!("&1"), Out)
        )))
    );
    parse_eq!(
        "foo >>bar",
        cs!(p!(e!(w!("foo") ; r!(1, w!("bar"), Append))))
    );
    parse_eq!(
        "foo >> bar",
        cs!(p!(e!(w!("foo") ; r!(1, w!("bar"), Append))))
    );
    parse_eq!(
        "foo > 'bar baz'",
        cs!(p!(e!(w!("foo") ; r!(1, w!(wps!("bar baz")), Out))))
    );
}

#[test]
fn test_escape() {
    parse_eq!("foo\\ bar", cs!(p!(e!(w!("foo bar")))));
    parse_eq!("'foo\\ bar'", cs!(p!(e!(w!(wps!("foo\\ bar"))))));
    parse_eq!("\"foo\\ bar\"", cs!(p!(e!(w!(wpd!("foo bar"))))));
    parse_eq!("\"foo\\\"bar\"", cs!(p!(e!(w!(wpd!("foo\"bar"))))));
    parse_eq!("'foo\\'bar\\\\'", cs!(p!(e!(w!(wps!("foo'bar\\"))))));
    parse_eq!(
        "foo > bar\\ baz",
        cs!(p!(e!(w!("foo") ; r!(1, w!("bar baz"), Out))))
    );
}

#[test]
fn test_parts() {
    parse_eq!(
        "echo \"$HOME/bin\"",
        cs!(p!(e!(w!("echo"), w!(wpv!("HOME"), wpd!("/bin")))))
    );
    parse_eq!(
        "echo $HOME/bin",
        cs!(p!(e!(w!("echo"), w!(wpv!("HOME"), wpb!("/bin")))))
    );
    parse_eq!(
        "echo '$HOME/bin'",
        cs!(p!(e!(w!("echo"), w!(wps!("$HOME/bin")))))
    );
    parse_eq!(
        "echo \"foo\"\"bar\"",
        cs!(p!(e!(w!("echo"), w!(wpd!("foo"), wpd!("bar")))))
    );
    parse_eq!(
        "echo $foo$bar$baz",
        cs!(p!(e!(
            w!("echo"),
            w!(wpv!("foo"), wpv!("bar"), wpv!("baz"))
        )))
    );
    parse_eq!(
        "perl -E'say \"foo\"'",
        cs!(p!(e!(w!("perl"), w!(wpb!("-E"), wps!("say \"foo\"")))))
    );
}
