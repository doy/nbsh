use std::os::unix::process::ExitStatusExt as _;

pub struct Env {
    latest_status: async_std::process::ExitStatus,
}

impl Env {
    pub fn new(code: i32) -> Self {
        Self {
            latest_status: async_std::process::ExitStatus::from_raw(
                code << 8,
            ),
        }
    }

    pub fn set_status(&mut self, status: async_std::process::ExitStatus) {
        self.latest_status = status;
    }

    pub fn latest_status(&self) -> &async_std::process::ExitStatus {
        &self.latest_status
    }
}
