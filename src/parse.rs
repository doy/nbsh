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
    fn build_ast(pair: pest::iterators::Pair<Rule>) -> Self {
        assert!(matches!(pair.as_rule(), Rule::word));
        let word = pair.into_inner().next().unwrap();
        assert!(matches!(
            word.as_rule(),
            Rule::bareword | Rule::single_string | Rule::double_string
        ));
        Self {
            word: word.as_str().to_string(),
            interpolate: matches!(
                word.as_rule(),
                Rule::bareword | Rule::double_string
            ),
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
        let exe = Word::build_ast(iter.next().unwrap());
        let args = iter.map(Word::build_ast).collect();
        Self { exe, args }
    }

    pub fn exe(&self) -> &str {
        &self.exe.word
    }

    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(|arg| arg.word.as_ref())
    }

    pub fn shift(&self) -> Self {
        let mut new = self.clone();
        let new_exe = new.args.remove(0);
        new.exe = new_exe;
        new
    }
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    exes: Vec<Exe>,
    input_string: String,
}

impl Pipeline {
    pub fn parse(pipeline: &str) -> Result<Self, Error> {
        Ok(Self::build_ast(
            Shell::parse(Rule::pipeline, pipeline)
                .map_err(|e| Error::new(pipeline, anyhow::anyhow!(e)))?
                .next()
                .unwrap(),
        ))
    }

    pub fn exes(&self) -> &[Exe] {
        &self.exes
    }

    pub fn input_string(&self) -> &str {
        &self.input_string
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

#[derive(Debug, Clone)]
pub struct Commands {
    pipelines: Vec<Pipeline>,
    input_string: String,
}

impl Commands {
    pub fn parse(full_cmd: &str) -> Result<Self, Error> {
        Ok(Self::build_ast(
            Shell::parse(Rule::line, full_cmd)
                .map_err(|e| Error::new(full_cmd, anyhow::anyhow!(e)))?
                .next()
                .unwrap()
                .into_inner()
                .next()
                .unwrap(),
        ))
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

#[derive(Debug)]
pub struct Error {
    input: String,
    e: anyhow::Error,
}

impl Error {
    fn new(input: &str, e: anyhow::Error) -> Self {
        Self {
            input: input.to_string(),
            e,
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn error(&self) -> &anyhow::Error {
        &self.e
    }
}
