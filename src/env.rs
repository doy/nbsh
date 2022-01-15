use crate::prelude::*;

use serde::Deserialize as _;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Env {
    V0(V0),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct V0 {
    idx: usize,
    #[serde(
        serialize_with = "serialize_status",
        deserialize_with = "deserialize_status"
    )]
    latest_status: async_std::process::ExitStatus,
    pwd: std::path::PathBuf,
    vars: std::collections::HashMap<std::ffi::OsString, std::ffi::OsString>,
}

impl Env {
    pub fn new() -> anyhow::Result<Self> {
        let mut vars: std::collections::HashMap<
            std::ffi::OsString,
            std::ffi::OsString,
        > = std::env::vars_os().collect();
        vars.insert("SHELL".into(), std::env::current_exe()?.into());
        vars.insert("TERM".into(), "screen".into());
        Ok(Self::V0(V0 {
            idx: 0,
            latest_status: std::process::ExitStatus::from_raw(0),
            pwd: std::env::current_dir()?,
            vars,
        }))
    }

    pub fn idx(&self) -> usize {
        match self {
            Self::V0(env) => env.idx,
        }
    }

    pub fn set_idx(&mut self, idx: usize) {
        match self {
            Self::V0(env) => env.idx = idx,
        }
    }

    pub fn latest_status(&self) -> &async_std::process::ExitStatus {
        match self {
            Self::V0(env) => &env.latest_status,
        }
    }

    pub fn set_status(&mut self, status: async_std::process::ExitStatus) {
        match self {
            Self::V0(env) => {
                env.latest_status = status;
            }
        }
    }

    pub fn current_dir(&self) -> &std::path::Path {
        match self {
            Self::V0(env) => &env.pwd,
        }
    }

    pub fn set_current_dir(&mut self, pwd: std::path::PathBuf) {
        match self {
            Self::V0(env) => {
                env.pwd = pwd;
            }
        }
    }

    pub fn var(&self, k: &str) -> String {
        match self {
            Self::V0(env) => self.special_var(k).unwrap_or_else(|| {
                env.vars.get(std::ffi::OsStr::new(k)).map_or_else(
                    || "".to_string(),
                    |v| v.to_str().unwrap().to_string(),
                )
            }),
        }
    }

    pub fn set_var<T: Into<std::ffi::OsString>>(&mut self, k: T, v: T) {
        match self {
            Self::V0(env) => {
                env.vars.insert(k.into(), v.into());
            }
        }
    }

    pub fn set_vars(
        &mut self,
        it: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    ) {
        match self {
            Self::V0(env) => {
                env.vars = it.collect();
            }
        }
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

    pub fn update(
        &mut self,
        status: std::process::ExitStatus,
    ) -> anyhow::Result<()> {
        self.set_status(status);
        self.set_current_dir(std::env::current_dir()?);
        self.set_vars(std::env::vars_os());
        Ok(())
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        bincode::deserialize(bytes).unwrap()
    }

    fn special_var(&self, k: &str) -> Option<String> {
        match self {
            Self::V0(env) => Some(match k {
                "$" => crate::info::pid(),
                "?" => {
                    let status = env.latest_status;
                    status
                        .signal()
                        .map_or_else(
                            || status.code().unwrap(),
                            |signal| signal + 128,
                        )
                        .to_string()
                }
                _ => return None,
            }),
        }
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn serialize_status<S>(
    status: &std::process::ExitStatus,
    s: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let code: u16 = status.code().unwrap_or(0).try_into().unwrap();
    let signal: u16 = status.signal().unwrap_or(0).try_into().unwrap();
    s.serialize_u16((code << 8) | signal)
}

fn deserialize_status<'de, D>(
    d: D,
) -> Result<std::process::ExitStatus, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let status = u16::deserialize(d)?;
    Ok(std::process::ExitStatus::from_raw(i32::from(status)))
}
