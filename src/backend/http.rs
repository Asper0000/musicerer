use std::time::Duration;

/// Builds an agent with a deadline covering the complete request, including
/// redirects and response-body reads. This preserves ureq 2's `.timeout(...)`
/// behavior for metadata and thumbnail requests.
pub(crate) fn agent_with_global_timeout(
    user_agent: Option<&str>,
    timeout: Duration,
) -> ureq::Agent {
    let builder = ureq::Agent::config_builder().timeout_global(Some(timeout));
    match user_agent {
        Some(value) => builder.user_agent(value).build().into(),
        None => builder.build().into(),
    }
}

/// Builds an agent with the separate connect and per-read limits used by the
/// updater. This preserves the old `.timeout_connect(...)` and
/// `.timeout_read(...)` configuration without imposing a whole-download limit.
pub(crate) fn agent_with_socket_timeouts(
    user_agent: Option<&str>,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> ureq::Agent {
    let builder = ureq::Agent::config_builder()
        .timeout_connect(Some(connect_timeout))
        .timeout_per_read(Some(read_timeout));
    match user_agent {
        Some(value) => builder.user_agent(value).build().into(),
        None => builder.build().into(),
    }
}
