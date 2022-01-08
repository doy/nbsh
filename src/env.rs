use crate::prelude::*;

use serde::Deserialize as _;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Env {
    V0(V0),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct V0 {
    pipeline: Option<String>,
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
    pub fn new() -> Self {
        Self::V0(V0 {
            pipeline: None,
            idx: 0,
            latest_status: std::process::ExitStatus::from_raw(0),
            pwd: std::env::current_dir().unwrap(),
            vars: std::env::vars_os().collect(),
        })
    }

    pub fn pipeline(&self) -> Option<&str> {
        match self {
            Self::V0(env) => env.pipeline.as_deref(),
        }
    }

    pub fn set_pipeline(&mut self, pipeline: String) {
        match self {
            Self::V0(env) => {
                env.pipeline = Some(pipeline);
            }
        }
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
            Self::V0(env) => {
                env.vars.get(std::ffi::OsStr::new(k)).map_or_else(
                    || "".to_string(),
                    |v| v.to_str().unwrap().to_string(),
                )
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

    pub fn update(&mut self) -> anyhow::Result<()> {
        self.set_current_dir(std::env::current_dir()?);
        Ok(())
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        bincode::deserialize(bytes).unwrap()
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
