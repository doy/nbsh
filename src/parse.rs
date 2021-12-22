pub struct Word {
    word: String,
    interpolate: bool,
}

impl Word {
    fn new(word: String) -> Self {
        Self {
            word,
            interpolate: true,
        }
    }

    fn literal(word: String) -> Self {
        Self {
            word,
            interpolate: false,
        }
    }
}

pub struct Exe {
    exe: Word,
    args: Vec<Word>,
}

impl Exe {
    pub fn exe(&self) -> &str {
        &self.exe.word
    }

    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(|arg| arg.word.as_ref())
    }
}

pub enum Command {
    Exe(Exe),
    And(Vec<Command>),
    Or(Vec<Command>),
    Both(Vec<Command>),
    Pipe(Vec<Command>),
}

impl Command {
    pub fn parse(full_cmd: &str) -> Self {
        let mut parts = full_cmd.split(' ');
        let cmd = parts.next().unwrap();
        Self::Exe(Exe {
            exe: Word::new(cmd.to_string()),
            args: parts
                .map(std::string::ToString::to_string)
                .map(Word::new)
                .collect(),
        })
    }
}
