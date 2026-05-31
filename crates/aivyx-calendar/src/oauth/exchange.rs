//! Re-export shim — exchange helpers now live in
//! [`aivyx_google_oauth::exchange`] (Phase 129 Task 2 lift).

pub use aivyx_google_oauth::exchange::{
    exchange_code, refresh_access_token, ExchangeError, GOOGLE_AUTH_ENDPOINT,
    GOOGLE_TOKEN_ENDPOINT,
};
