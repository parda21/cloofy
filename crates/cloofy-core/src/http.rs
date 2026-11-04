//! Settings shared by every HTTP client cloofy creates.

use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const KEEPALIVE: Duration = Duration::from_secs(30);

/// A client builder with timeouts. Without them a connection that died while the laptop slept
/// hangs a request until TCP gives up, which can take minutes.
pub fn client(user_agent: &str) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .tcp_keepalive(KEEPALIVE)
}
