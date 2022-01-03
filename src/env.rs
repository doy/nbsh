use serde::Deserialize as _;
use std::os::unix::process::ExitStatusExt as _;

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Env {
    V0(V0),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct V0 {
    pipeline: Option<String>,
    #[serde(
        serialize_with = "serialize_status",
        deserialize_with = "deserialize_status"
    )]
    latest_status: async_std::process::ExitStatus,
}

impl Env {
    pub fn new() -> Self {
        Self::V0(V0 {
            pipeline: None,
            latest_status: std::process::ExitStatus::from_raw(0),
        })
    }

    pub fn set_pipeline(&mut self, pipeline: String) {
        match self {
            Self::V0(env) => {
                env.pipeline = Some(pipeline);
            }
        }
    }

    pub fn set_status(&mut self, status: async_std::process::ExitStatus) {
        match self {
            Self::V0(env) => {
                env.latest_status = status;
            }
        }
    }

    pub fn pipeline(&self) -> Option<&str> {
        match self {
            Self::V0(env) => env.pipeline.as_deref(),
        }
    }

    pub fn latest_status(&self) -> &async_std::process::ExitStatus {
        match self {
            Self::V0(env) => &env.latest_status,
        }
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
