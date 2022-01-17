use crate::prelude::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Env {
    V0(V0),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct V0 {
    pwd: std::path::PathBuf,
    vars: std::collections::HashMap<std::ffi::OsString, std::ffi::OsString>,
}

const __NBSH_IDX: &str = "__NBSH_IDX";
const __NBSH_LATEST_STATUS: &str = "__NBSH_LATEST_STATUS";
const __NBSH_PREV_PWD: &str = "__NBSH_PREV_PWD";

impl Env {
    pub fn new() -> anyhow::Result<Self> {
        let pwd = std::env::current_dir()?;
        Ok(Self::V0(V0 {
            pwd: pwd.clone(),
            vars: std::env::vars_os()
                .chain(
                    [
                        (__NBSH_IDX.into(), "0".into()),
                        (__NBSH_LATEST_STATUS.into(), "0".into()),
                        (__NBSH_PREV_PWD.into(), pwd.into()),
                    ]
                    .into_iter(),
                )
                .collect(),
        }))
    }

    pub fn pwd(&self) -> &std::path::Path {
        match self {
            Self::V0(env) => &env.pwd,
        }
    }

    pub fn var(&self, k: &str) -> Option<String> {
        match self {
            Self::V0(env) => self.special_var(k).or_else(|| {
                env.vars
                    .get(std::ffi::OsStr::new(k))
                    .map(|v| v.to_str().unwrap().to_string())
            }),
        }
    }

    pub fn set_var<
        K: Into<std::ffi::OsString>,
        V: Into<std::ffi::OsString>,
    >(
        &mut self,
        k: K,
        v: V,
    ) {
        match self {
            Self::V0(env) => {
                env.vars.insert(k.into(), v.into());
            }
        }
    }

    pub fn idx(&self) -> usize {
        self.var(__NBSH_IDX).unwrap().parse().unwrap()
    }

    pub fn set_idx(&mut self, idx: usize) {
        self.set_var(__NBSH_IDX, format!("{}", idx));
    }

    pub fn latest_status(&self) -> std::process::ExitStatus {
        std::process::ExitStatus::from_raw(
            self.var(__NBSH_LATEST_STATUS).unwrap().parse().unwrap(),
        )
    }

    pub fn set_status(&mut self, status: std::process::ExitStatus) {
        self.set_var(
            __NBSH_LATEST_STATUS,
            format!(
                "{}",
                (status.code().unwrap_or(0) << 8)
                    | status.signal().unwrap_or(0)
            ),
        );
    }

    pub fn prev_pwd(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(self.var(__NBSH_PREV_PWD).unwrap())
    }

    pub fn set_prev_pwd(&mut self, prev_pwd: std::path::PathBuf) {
        self.set_var(__NBSH_PREV_PWD, prev_pwd);
    }

    pub fn apply(&self, cmd: &mut pty_process::Command) {
        match self {
            Self::V0(env) => {
                cmd.current_dir(&env.pwd);
                cmd.env_clear();
                cmd.envs(env.vars.iter());
            }
        }
    }

    pub fn update(&mut self) -> anyhow::Result<()> {
        let idx = self.idx();
        let status = self.latest_status();
        let prev_pwd = self.prev_pwd();
        *self = Self::new()?;
        self.set_idx(idx);
        self.set_status(status);
        self.set_prev_pwd(prev_pwd);
        Ok(())
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        bincode::deserialize(bytes).unwrap()
    }

    fn special_var(&self, k: &str) -> Option<String> {
        Some(match k {
            "$" => crate::info::pid(),
            "?" => {
                let status = self.latest_status();
                status
                    .signal()
                    .map_or_else(
                        || status.code().unwrap(),
                        |signal| signal + 128,
                    )
                    .to_string()
            }
            _ => return None,
        })
    }
}
