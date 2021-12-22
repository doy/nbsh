use pest::Parser as _;

#[derive(pest_derive::Parser)]
#[grammar = "shell.pest"]
struct Shell;

#[derive(Debug, Clone)]
pub struct Word {
    word: String,
    interpolate: bool,
}

impl Word {
    fn new(word: &str) -> Self {
        Self {
            word: word.to_string(),
            interpolate: true,
        }
    }

    fn literal(word: &str) -> Self {
        Self {
            word: word.to_string(),
            interpolate: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Exe {
    exe: Word,
    args: Vec<Word>,
    original: String,
}

impl Exe {
    fn parse(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::exe));
        let original = pair.as_str().to_string();
        let mut iter = pair.into_inner();
        let exe = Word::new(iter.next().unwrap().as_str());
        let args = iter.map(|word| Word::new(word.as_str())).collect();
        Self {
            exe,
            args,
            original,
        }
    }

    pub fn exe(&self) -> &str {
        &self.exe.word
    }

    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(|arg| arg.word.as_ref())
    }

    pub fn input_string(&self) -> String {
        self.original.clone()
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Exe(Exe),
    And(Exe, Box<Command>),
    Or(Exe, Box<Command>),
    Both(Exe, Box<Command>),
    Pipe(Exe, Box<Command>),
}

impl Command {
    pub fn parse(full_cmd: &str) -> Self {
        Self::build_ast(
            Shell::parse(Rule::line, full_cmd)
                .unwrap()
                .next()
                .unwrap()
                .into_inner(),
        )
    }

    pub fn input_string(&self) -> String {
        match self {
            Self::Exe(exe) => exe.input_string(),
            Self::And(exe, command) => format!(
                "{} && {}",
                exe.input_string(),
                command.input_string()
            ),
            Self::Or(exe, command) => format!(
                "{} || {}",
                exe.input_string(),
                command.input_string()
            ),
            Self::Both(exe, command) => {
                format!("{}; {}", exe.input_string(), command.input_string())
            }
            Self::Pipe(exe, command) => {
                format!("{} | {}", exe.input_string(), command.input_string())
            }
        }
    }

    fn build_ast(mut pairs: pest::iterators::Pairs<Rule>) -> Self {
        let command = pairs.next().unwrap();
        assert!(matches!(command.as_rule(), Rule::command));
        let mut inner = command.into_inner();
        let exe = inner.next().unwrap();
        let exe = Exe::parse(exe);
        if let Some(rest) = inner.next() {
            let rule = rest.as_rule();
            let ast = Self::build_ast(rest.into_inner());
            match rule {
                Rule::and => Self::And(exe, Box::new(ast)),
                Rule::or => Self::Or(exe, Box::new(ast)),
                Rule::both => Self::Both(exe, Box::new(ast)),
                Rule::pipe => Self::Pipe(exe, Box::new(ast)),
                _ => unreachable!(),
            }
        } else {
            Self::Exe(exe)
        }
    }
}
