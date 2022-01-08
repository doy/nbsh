use super::*;

impl From<Pipeline> for Command {
    fn from(pipeline: Pipeline) -> Self {
        Self::Pipeline(pipeline)
    }
}

macro_rules! cs {
    ($input_string:expr, $($commands:expr),*) => {
        Commands {
            commands: [$($commands),*]
                .into_iter()
                .map(|c| c.into())
                .collect(),
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
    parse_eq!("foo", cs!("foo", p!("foo", e!(w!("foo")))));
    parse_eq!(
        "foo bar",
        cs!("foo bar", p!("foo bar", e!(w!("foo"), w!("bar"))))
    );
    parse_eq!(
        "foo bar baz",
        cs!(
            "foo bar baz",
            p!("foo bar baz", e!(w!("foo"), w!("bar"), w!("baz")))
        )
    );
    parse_eq!(
        "foo | bar",
        cs!("foo | bar", p!("foo | bar", e!(w!("foo")), e!(w!("bar"))))
    );
    parse_eq!(
        "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar",
        cs!(
            "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar",
            p!("command ls", e!(w!("command"), w!("ls"))),
            p!(
                "perl -E 'say foo' | tr a-z A-Z",
                e!(w!("perl"), w!("-E"), w!(wps!("say foo"))),
                e!(w!("tr"), w!("a-z"), w!("A-Z"))
            ),
            p!("builtin echo bar", e!(w!("builtin"), w!("echo"), w!("bar")))
        )
    );
}

#[test]
fn test_whitespace() {
    parse_eq!("   foo    ", cs!("foo", p!("foo", e!(w!("foo")))));
    parse_eq!(
        "   foo    # this is a comment",
        cs!("foo", p!("foo", e!(w!("foo"))))
    );
    parse_eq!("foo#comment", cs!("foo", p!("foo", e!(w!("foo")))));
    parse_eq!(
        "foo;bar|baz;quux#comment",
        cs!(
            "foo;bar|baz;quux",
            p!("foo", e!(w!("foo"))),
            p!("bar|baz", e!(w!("bar")), e!(w!("baz"))),
            p!("quux", e!(w!("quux")))
        )
    );
    parse_eq!(
        "foo    | bar  ",
        cs!(
            "foo    | bar",
            p!("foo    | bar", e!(w!("foo")), e!(w!("bar")))
        )
    );
    parse_eq!(
        "  abc def  ghi   |jkl mno|   pqr stu; vwxyz  # comment",
        cs!(
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
        cs!(
            "foo 'bar # baz' \"quux # not a comment\"",
            p!(
                "foo 'bar # baz' \"quux # not a comment\"",
                e!(
                    w!("foo"),
                    w!(wps!("bar # baz")),
                    w!(wpd!("quux # not a comment"))
                )
            )
        )
    );
}

#[test]
fn test_redirect() {
    parse_eq!(
        "foo > bar",
        cs!(
            "foo > bar",
            p!("foo > bar", e!(w!("foo") ; r!(1, w!("bar"), Out)))
        )
    );
    parse_eq!(
        "foo <bar",
        cs!(
            "foo <bar",
            p!("foo <bar", e!(w!("foo") ; r!(0, w!("bar"), In)))
        )
    );
    parse_eq!(
        "foo > /dev/null 2>&1",
        cs!(
            "foo > /dev/null 2>&1",
            p!(
                "foo > /dev/null 2>&1",
                e!(
                    w!("foo") ;
                    r!(1, w!("/dev/null"), Out), r!(2, w!("&1"), Out)
                )
            )
        )
    );
    parse_eq!(
        "foo >>bar",
        cs!(
            "foo >>bar",
            p!("foo >>bar", e!(w!("foo") ; r!(1, w!("bar"), Append)))
        )
    );
    parse_eq!(
        "foo >> bar",
        cs!(
            "foo >> bar",
            p!("foo >> bar", e!(w!("foo") ; r!(1, w!("bar"), Append)))
        )
    );
    parse_eq!(
        "foo > 'bar baz'",
        cs!(
            "foo > 'bar baz'",
            p!(
                "foo > 'bar baz'",
                e!(w!("foo") ; r!(1, w!(wps!("bar baz")), Out))
            )
        )
    );
}

#[test]
fn test_escape() {
    parse_eq!(
        "foo\\ bar",
        cs!("foo\\ bar", p!("foo\\ bar", e!(w!("foo bar"))))
    );
    parse_eq!(
        "'foo\\ bar'",
        cs!("'foo\\ bar'", p!("'foo\\ bar'", e!(w!(wps!("foo\\ bar")))))
    );
    parse_eq!(
        "\"foo\\ bar\"",
        cs!(
            "\"foo\\ bar\"",
            p!("\"foo\\ bar\"", e!(w!(wpd!("foo bar"))))
        )
    );
    parse_eq!(
        "\"foo\\\"bar\"",
        cs!(
            "\"foo\\\"bar\"",
            p!("\"foo\\\"bar\"", e!(w!(wpd!("foo\"bar"))))
        )
    );
    parse_eq!(
        "'foo\\'bar\\\\'",
        cs!(
            "'foo\\'bar\\\\'",
            p!("'foo\\'bar\\\\'", e!(w!(wps!("foo'bar\\"))))
        )
    );
    parse_eq!(
        "foo > bar\\ baz",
        cs!(
            "foo > bar\\ baz",
            p!("foo > bar\\ baz", e!(w!("foo") ; r!(1, w!("bar baz"), Out)))
        )
    );
}

#[test]
fn test_parts() {
    parse_eq!(
        "echo \"$HOME/bin\"",
        cs!(
            "echo \"$HOME/bin\"",
            p!(
                "echo \"$HOME/bin\"",
                e!(w!("echo"), w!(wpv!("HOME"), wpd!("/bin")))
            )
        )
    );
    parse_eq!(
        "echo $HOME/bin",
        cs!(
            "echo $HOME/bin",
            p!(
                "echo $HOME/bin",
                e!(w!("echo"), w!(wpv!("HOME"), wpb!("/bin")))
            )
        )
    );
    parse_eq!(
        "echo '$HOME/bin'",
        cs!(
            "echo '$HOME/bin'",
            p!("echo '$HOME/bin'", e!(w!("echo"), w!(wps!("$HOME/bin"))))
        )
    );
    parse_eq!(
        "echo \"foo\"\"bar\"",
        cs!(
            "echo \"foo\"\"bar\"",
            p!(
                "echo \"foo\"\"bar\"",
                e!(w!("echo"), w!(wpd!("foo"), wpd!("bar")))
            )
        )
    );
    parse_eq!(
        "echo $foo$bar$baz",
        cs!(
            "echo $foo$bar$baz",
            p!(
                "echo $foo$bar$baz",
                e!(w!("echo"), w!(wpv!("foo"), wpv!("bar"), wpv!("baz")))
            )
        )
    );
    parse_eq!(
        "perl -E'say \"foo\"'",
        cs!(
            "perl -E'say \"foo\"'",
            p!(
                "perl -E'say \"foo\"'",
                e!(w!("perl"), w!(wpb!("-E"), wps!("say \"foo\"")))
            )
        )
    );
}
