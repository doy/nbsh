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
}

impl Exe {
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::exe));
        let mut iter = pair.into_inner();
        let exe = Word::new(iter.next().unwrap().as_str());
        let args = iter.map(|word| Word::new(word.as_str())).collect();
        Self { exe, args }
    }

    pub fn exe(&self) -> &str {
        &self.exe.word
    }

    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(|arg| arg.word.as_ref())
    }
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    exes: Vec<Exe>,
}

impl Pipeline {
    pub fn exes(&self) -> &[Exe] {
        &self.exes
    }

    fn build_ast(pipeline: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pipeline.as_rule(), Rule::pipeline));
        Self {
            exes: pipeline.into_inner().map(Exe::build_ast).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Commands {
    pipelines: Vec<Pipeline>,
    input_string: String,
}

impl Commands {
    pub fn parse(full_cmd: &str) -> Self {
        Self::build_ast(
            Shell::parse(Rule::line, full_cmd)
                .unwrap()
                .next()
                .unwrap()
                .into_inner()
                .next()
                .unwrap(),
        )
    }

    pub fn pipelines(&self) -> &[Pipeline] {
        &self.pipelines
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
