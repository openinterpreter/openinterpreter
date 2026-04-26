use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use codex_app_server_client::AppServerClient;
use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessClientStartArgs;
use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;

pub(crate) enum ExecAppServerTarget {
    InProcess(InProcessClientStartArgs),
    Remote(RemoteAppServerConnectArgs),
}

impl ExecAppServerTarget {
    pub(crate) async fn connect(self) -> Result<AppServerClient> {
        match self {
            Self::InProcess(args) => Ok(AppServerClient::InProcess(
                InProcessAppServerClient::start(args).await.map_err(|err| {
                    anyhow::anyhow!("failed to initialize in-process app-server client: {err}")
                })?,
            )),
            Self::Remote(args) => Ok(AppServerClient::Remote(
                RemoteAppServerClient::connect(args).await.map_err(|err| {
                    anyhow::anyhow!("failed to connect to remote app-server client: {err}")
                })?,
            )),
        }
    }
}

pub(crate) fn exec_app_server_target(
    in_process_start_args: InProcessClientStartArgs,
    remote: Option<String>,
    remote_auth_token_env: Option<String>,
) -> Result<ExecAppServerTarget> {
    match remote {
        Some(websocket_url) => Ok(ExecAppServerTarget::Remote(RemoteAppServerConnectArgs {
            websocket_url,
            auth_token: remote_auth_token_env
                .as_deref()
                .map(read_remote_auth_token_from_env_var)
                .transpose()?,
            client_name: "codex_exec".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            experimental_api: true,
            opt_out_notification_methods: Vec::new(),
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })),
        None => {
            if remote_auth_token_env.is_some() {
                bail!("`--remote-auth-token-env` requires `--remote`.")
            }
            Ok(ExecAppServerTarget::InProcess(in_process_start_args))
        }
    }
}

fn read_remote_auth_token_from_env_var(env_var_name: &str) -> Result<String> {
    let token = std::env::var(env_var_name).with_context(|| {
        format!("failed to read remote auth token from environment variable `{env_var_name}`")
    })?;
    if token.trim().is_empty() {
        bail!("environment variable `{env_var_name}` contained an empty auth token");
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn lagged_event_warning_message_is_transport_neutral() {
        assert_eq!(
            lagged_event_warning_message(/*skipped*/ 7),
            "app-server event stream lagged; dropped 7 events".to_string()
        );
    }

    #[test]
    fn remote_auth_token_reader_rejects_empty_value() {
        unsafe {
            std::env::set_var("CODEX_EXEC_EMPTY_TOKEN", "");
        }

        let err = read_remote_auth_token_from_env_var("CODEX_EXEC_EMPTY_TOKEN")
            .expect_err("empty token should fail");
        assert!(err.to_string().contains("contained an empty auth token"));
    }
}
