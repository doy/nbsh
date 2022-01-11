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
    ($span:expr, $($exes:expr),*) => {
        Pipeline {
            exes: vec![$($exes),*],
            span: $span,
        }
    };
}

macro_rules! ep {
    ($($exes:expr),*) => {
        super::super::Pipeline {
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

macro_rules! ee {
    ($exe:expr) => {
        super::super::Exe {
            exe: std::path::PathBuf::from($exe.to_string()),
            args: vec![],
            redirects: vec![],
        }
    };
    ($exe:expr, $($args:expr),*) => {
        super::super::Exe {
            exe: std::path::PathBuf::from($exe.to_string()),
            args: [$($args),*]
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
            redirects: vec![],
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

macro_rules! wpa {
    ($($word:expr),*) => {
        WordPart::Alternation(vec![$($word),*])
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

macro_rules! eval_eq {
    ($line:literal, $env:expr, $($evaled:expr),*) => {{
        let ast = Commands::parse($line).unwrap();
        let mut expected: Vec<super::super::Pipeline>
            = vec![$($evaled),*];
        for command in ast.commands {
            let pipeline = match command {
                Command::Pipeline(p)
                    | Command::If(p)
                    | Command::While(p) => p,
                _ => continue,
            };
            assert_eq!(pipeline.eval(&$env).unwrap(), expected.remove(0));
        }
    }};
}

macro_rules! eval_fails {
    ($line:literal, $env:expr) => {{
        let ast = Commands::parse($line).unwrap();
        let mut fail = false;
        for command in ast.commands {
            let pipeline = match command {
                Command::Pipeline(p) | Command::If(p) | Command::While(p) => {
                    p
                }
                _ => continue,
            };
            if pipeline.eval(&$env).is_err() {
                fail = true;
            }
        }
        assert!(fail)
    }};
}

#[test]
fn test_basic() {
    parse_eq!("foo", cs!(p!((0, 3), e!(w!("foo")))));
    parse_eq!("foo bar", cs!(p!((0, 7), e!(w!("foo"), w!("bar")))));
    parse_eq!(
        "foo bar baz",
        cs!(p!((0, 11), e!(w!("foo"), w!("bar"), w!("baz"))))
    );
    parse_eq!("foo | bar", cs!(p!((0, 9), e!(w!("foo")), e!(w!("bar")))));
    parse_eq!(
        "command ls; perl -E 'say foo' | tr a-z A-Z; builtin echo bar",
        cs!(
            p!((0, 10), e!(w!("command"), w!("ls"))),
            p!(
                (12, 42),
                e!(w!("perl"), w!("-E"), w!(wps!("say foo"))),
                e!(w!("tr"), w!("a-z"), w!("A-Z"))
            ),
            p!((44, 60), e!(w!("builtin"), w!("echo"), w!("bar")))
        )
    );
}

#[test]
fn test_whitespace() {
    parse_eq!("   foo    ", cs!(p!((3, 6), e!(w!("foo")))));
    parse_eq!(
        "   foo    # this is a comment",
        cs!(p!((3, 6), e!(w!("foo"))))
    );
    parse_eq!("foo#comment", cs!(p!((0, 3), e!(w!("foo")))));
    parse_eq!(
        "foo;bar|baz;quux#comment",
        cs!(
            p!((0, 3), e!(w!("foo"))),
            p!((4, 11), e!(w!("bar")), e!(w!("baz"))),
            p!((12, 16), e!(w!("quux")))
        )
    );
    parse_eq!(
        "foo    | bar  ",
        cs!(p!((0, 12), e!(w!("foo")), e!(w!("bar"))))
    );
    parse_eq!(
        "  abc def  ghi   |jkl mno|   pqr stu; vwxyz  # comment",
        cs!(
            p!(
                (2, 36),
                e!(w!("abc"), w!("def"), w!("ghi")),
                e!(w!("jkl"), w!("mno")),
                e!(w!("pqr"), w!("stu"))
            ),
            p!((38, 43), e!(w!("vwxyz")))
        )
    );
    parse_eq!(
        "foo 'bar # baz' \"quux # not a comment\" # comment",
        cs!(p!(
            (0, 38),
            e!(
                w!("foo"),
                w!(wps!("bar # baz")),
                w!(wpd!("quux # not a comment"))
            )
        ))
    );
}

#[test]
fn test_redirect() {
    parse_eq!(
        "foo > bar",
        cs!(p!((0, 9), e!(w!("foo") ; r!(1, w!("bar"), Out))))
    );
    parse_eq!(
        "foo <bar",
        cs!(p!((0, 8), e!(w!("foo") ; r!(0, w!("bar"), In))))
    );
    parse_eq!(
        "foo > /dev/null 2>&1",
        cs!(p!(
            (0, 20),
            e!(
                w!("foo") ;
                r!(1, w!("/dev/null"), Out), r!(2, w!("&1"), Out)
            )
        ))
    );
    parse_eq!(
        "foo >>bar",
        cs!(p!((0, 9), e!(w!("foo") ; r!(1, w!("bar"), Append))))
    );
    parse_eq!(
        "foo >> bar",
        cs!(p!((0, 10), e!(w!("foo") ; r!(1, w!("bar"), Append))))
    );
    parse_eq!(
        "foo > 'bar baz'",
        cs!(p!((0, 15), e!(w!("foo") ; r!(1, w!(wps!("bar baz")), Out))))
    );
}

#[test]
fn test_escape() {
    parse_eq!("foo\\ bar", cs!(p!((0, 8), e!(w!("foo bar")))));
    parse_eq!("'foo\\ bar'", cs!(p!((0, 10), e!(w!(wps!("foo\\ bar"))))));
    parse_eq!("\"foo\\ bar\"", cs!(p!((0, 10), e!(w!(wpd!("foo bar"))))));
    parse_eq!("\"foo\\\"bar\"", cs!(p!((0, 10), e!(w!(wpd!("foo\"bar"))))));
    parse_eq!(
        "'foo\\'bar\\\\'",
        cs!(p!((0, 12), e!(w!(wps!("foo'bar\\")))))
    );
    parse_eq!(
        "foo > bar\\ baz",
        cs!(p!((0, 14), e!(w!("foo") ; r!(1, w!("bar baz"), Out))))
    );
}

#[test]
fn test_parts() {
    parse_eq!(
        "echo \"$HOME/bin\"",
        cs!(p!((0, 16), e!(w!("echo"), w!(wpv!("HOME"), wpd!("/bin")))))
    );
    parse_eq!(
        "echo $HOME/bin",
        cs!(p!((0, 14), e!(w!("echo"), w!(wpv!("HOME"), wpb!("/bin")))))
    );
    parse_eq!(
        "echo '$HOME/bin'",
        cs!(p!((0, 16), e!(w!("echo"), w!(wps!("$HOME/bin")))))
    );
    parse_eq!(
        "echo \"foo\"\"bar\"",
        cs!(p!((0, 15), e!(w!("echo"), w!(wpd!("foo"), wpd!("bar")))))
    );
    parse_eq!(
        "echo $foo$bar$baz",
        cs!(p!(
            (0, 17),
            e!(w!("echo"), w!(wpv!("foo"), wpv!("bar"), wpv!("baz")))
        ))
    );
    parse_eq!(
        "perl -E'say \"foo\"'",
        cs!(p!(
            (0, 18),
            e!(w!("perl"), w!(wpb!("-E"), wps!("say \"foo\"")))
        ))
    );
}

#[test]
fn test_alternation() {
    parse_eq!(
        "echo {foo,bar}",
        cs!(p!((0, 14), e!(w!("echo"), w!(wpa!(w!("foo"), w!("bar"))))))
    );
    parse_eq!(
        "echo {foo,bar}.rs",
        cs!(p!(
            (0, 17),
            e!(w!("echo"), w!(wpa!(w!("foo"), w!("bar")), wpb!(".rs")))
        ))
    );
    parse_eq!(
        "echo {foo,bar,baz}.rs",
        cs!(p!(
            (0, 21),
            e!(
                w!("echo"),
                w!(wpa!(w!("foo"), w!("bar"), w!("baz")), wpb!(".rs"))
            )
        ))
    );
    parse_eq!(
        "echo {foo,}.rs",
        cs!(p!(
            (0, 14),
            e!(w!("echo"), w!(wpa!(w!("foo"), w!()), wpb!(".rs")))
        ))
    );
    parse_eq!(
        "echo {foo}",
        cs!(p!((0, 10), e!(w!("echo"), w!(wpa!(w!("foo"))))))
    );
    parse_eq!("echo {}", cs!(p!((0, 7), e!(w!("echo"), w!(wpa!(w!()))))));
    parse_eq!(
        "echo {foo,bar}.{rs,c}",
        cs!(p!(
            (0, 21),
            e!(
                w!("echo"),
                w!(
                    wpa!(w!("foo"), w!("bar")),
                    wpb!("."),
                    wpa!(w!("rs"), w!("c"))
                )
            )
        ))
    );
    parse_eq!(
        "echo {$foo,\"${HOME}/bin\"}.{'r'\"s\",c}",
        cs!(p!(
            (0, 36),
            e!(
                w!("echo"),
                w!(
                    wpa!(w!(wpv!("foo")), w!(wpv!("HOME"), wpd!("/bin"))),
                    wpb!("."),
                    wpa!(w!(wps!("r"), wpd!("s")), w!("c"))
                )
            )
        ))
    );
}

#[test]
fn test_eval_alternation() {
    let mut env = Env::new();
    env.set_var("HOME", "/home/test");
    env.set_var("foo", "value-of-foo");

    eval_eq!("echo {foo,bar}", env, ep!(ee!("echo", "foo", "bar")));
    eval_eq!(
        "echo {foo,bar}.rs",
        env,
        ep!(ee!("echo", "foo.rs", "bar.rs"))
    );
    eval_eq!(
        "echo {foo,bar,baz}.rs",
        env,
        ep!(ee!("echo", "foo.rs", "bar.rs", "baz.rs"))
    );
    eval_eq!("echo {foo,}.rs", env, ep!(ee!("echo", "foo.rs", ".rs")));
    eval_eq!("echo {foo}", env, ep!(ee!("echo", "foo")));
    eval_eq!("echo {}", env, ep!(ee!("echo", "")));
    eval_eq!(
        "echo {foo,bar}.{rs,c}",
        env,
        ep!(ee!("echo", "foo.rs", "foo.c", "bar.rs", "bar.c"))
    );
    eval_eq!(
        "echo {$foo,\"${HOME}/bin\"}.{'r'\"s\",c}",
        env,
        ep!(ee!(
            "echo",
            "value-of-foo.rs",
            "value-of-foo.c",
            "/home/test/bin.rs",
            "/home/test/bin.c"
        ))
    );
}

#[test]
fn test_eval_glob() {
    let env = Env::new();

    eval_eq!(
        "echo *.toml",
        env,
        ep!(ee!("echo", "Cargo.toml", "deny.toml"))
    );
    eval_eq!("echo .*.toml", env, ep!(ee!("echo", ".rustfmt.toml")));
    eval_eq!(
        "echo *.{lock,toml}",
        env,
        ep!(ee!("echo", "Cargo.lock", "Cargo.toml", "deny.toml"))
    );
    eval_eq!("echo foo]", env, ep!(ee!("echo", "foo]")));
    eval_fails!("echo foo[", env);
    eval_fails!("echo *.doesnotexist", env);
    eval_fails!("echo *.{toml,doesnotexist}", env);
}
