use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct LoginQr {
    pub url: String,
    pub qrcode_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginPollData {
    pub code: i64,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LoginPoll {
    WaitingForScan,
    WaitingForConfirmation,
    Success,
    Expired,
}

impl LoginPollData {
    pub fn status(&self) -> Option<LoginPoll> {
        match self.code {
            86101 => Some(LoginPoll::WaitingForScan),
            86090 => Some(LoginPoll::WaitingForConfirmation),
            0 => Some(LoginPoll::Success),
            86038 => Some(LoginPoll::Expired),
            _ => None,
        }
    }
}
