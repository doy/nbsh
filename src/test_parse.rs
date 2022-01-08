#[allow(clippy::wildcard_imports)]
use crate::parse::*;

impl From<std::os::unix::io::RawFd> for RedirectTarget {
    fn from(fd: std::os::unix::io::RawFd) -> Self {
        Self::Fd(fd)
    }
}

impl From<std::path::PathBuf> for RedirectTarget {
    fn from(path: std::path::PathBuf) -> Self {
        Self::File(path)
    }
}

#[allow(clippy::fallible_impl_from)]
impl From<&str> for RedirectTarget {
    fn from(path: &str) -> Self {
        Self::File(path.try_into().unwrap())
    }
}

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
    ($from:literal, $to:literal, $dir:ident) => {
        Redirect {
            from: $from,
            to: $to.into(),
            dir: Direction::$dir,
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
            p!("foo > bar", e!(w!("foo") ; r!(1, "bar", Out)))
        )
    );
    parse_eq!(
        "foo <bar",
        c!("foo <bar", p!("foo <bar", e!(w!("foo") ; r!(0, "bar", In))))
    );
    parse_eq!(
        "foo > /dev/null 2>&1",
        c!(
            "foo > /dev/null 2>&1",
            p!(
                "foo > /dev/null 2>&1",
                e!(w!("foo") ; r!(1, "/dev/null", Out), r!(2, 1, Out))
            )
        )
    );
    parse_eq!(
        "foo >>bar",
        c!(
            "foo >>bar",
            p!("foo >>bar", e!(w!("foo") ; r!(1, "bar", Append)))
        )
    );
    parse_eq!(
        "foo >> bar",
        c!(
            "foo >> bar",
            p!("foo >> bar", e!(w!("foo") ; r!(1, "bar", Append)))
        )
    );
    parse_eq!(
        "foo > 'bar baz'",
        c!(
            "foo > 'bar baz'",
            p!("foo > 'bar baz'", e!(w!("foo") ; r!(1, "bar baz", Out)))
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
            p!("foo > bar\\ baz", e!(w!("foo") ; r!(1, "bar baz", Out)))
        )
    );
}
