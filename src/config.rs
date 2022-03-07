use crate::prelude::*;

#[derive(serde::Deserialize, Default, Debug)]
pub struct Config {
    aliases:
        std::collections::HashMap<std::path::PathBuf, crate::parse::ast::Exe>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let file = crate::dirs::config_file();
        if std::fs::metadata(&file).is_ok() {
            Ok(toml::from_slice(&std::fs::read(&file)?)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn alias_for(
        &self,
        path: &std::path::Path,
    ) -> Option<&crate::parse::ast::Exe> {
        self.aliases.get(path)
    }
}
